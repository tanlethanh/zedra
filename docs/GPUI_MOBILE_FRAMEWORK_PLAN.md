# GPUI Mobile Framework Plan

## Vision

Transform GPUI into a complete mobile UI framework that rivals React Native in developer experience and native feel. Learn from React Native's architecture while leveraging Rust's performance advantages.

## Current State Assessment

### What Works Well

| Component | Status | Location |
|-----------|--------|----------|
| **StackNavigator** | ✅ Complete | `zedra-nav/stack.rs` |
| **TabNavigator** | ✅ Complete | `zedra-nav/tab.rs` |
| **DrawerHost** | ✅ Complete | `zedra-nav/drawer.rs` |
| **ModalHost** | ✅ Complete | `zedra-nav/modal.rs` |
| **Gesture Recognizers** | ✅ Complete | `zedra-gesture/` |
| **Touch-to-Scroll** | ✅ Working | `android_app.rs` |
| **Soft Keyboard** | ⚠️ Manual | JNI wired, no auto-trigger |
| **uniform_list** | ✅ Working | GPUI core |

### Critical Gaps

| Gap | Impact | Priority |
|-----|--------|----------|
| Momentum Scrolling | UX feels non-native | **P0** |
| TextInput Component | No reusable input field | **P0** |
| Gesture-Driven Animations | Drawer/sheet feel static | **P1** |
| Safe Area Insets | Content under notch | **P1** |
| Navigation Transitions | Jarring screen changes | **P2** |

---

## Architecture Overview

### React Native's Thread Model (What We're Learning From)

```
┌─────────────────────────────────────────────────────────────┐
│                     REACT NATIVE                            │
├─────────────────────────────────────────────────────────────┤
│  Touch Event                                                │
│      ↓                                                      │
│  Native UI Thread ──→ Gesture Handler ──→ Reanimated       │
│      │                     │                   │            │
│      │              (no JS bridge)      (60 FPS animations) │
│      ↓                     ↓                   ↓            │
│  JavaScript Thread ←── State Updates ←── Worklet Results   │
│      │                                                      │
│      ↓                                                      │
│  React Reconciler → Native Views                           │
└─────────────────────────────────────────────────────────────┘
```

### GPUI Mobile Architecture (Our Target)

```
┌─────────────────────────────────────────────────────────────┐
│                      GPUI MOBILE                            │
├─────────────────────────────────────────────────────────────┤
│  Touch Event (JNI)                                          │
│      ↓                                                      │
│  Command Queue ──→ Main Thread (Single-Threaded GPUI)      │
│      │                   │                                  │
│      │            ┌──────┴──────┐                          │
│      │            ↓             ↓                          │
│      │     GestureSystem   AnimationSystem                 │
│      │            │             │                          │
│      │            ↓             ↓                          │
│      │     Recognizers    SharedValues                     │
│      │            │             │                          │
│      │            └──────┬──────┘                          │
│      │                   ↓                                  │
│      │            Element Tree                              │
│      │                   ↓                                  │
│      └──────────→ Blade Renderer → Vulkan                  │
└─────────────────────────────────────────────────────────────┘
```

**Key Difference**: GPUI is single-threaded by design. Instead of worklets, we use Rust's zero-cost abstractions and shared mutable state via `Arc<Mutex<>>` or GPUI's `Model<T>`.

---

## Implementation Roadmap

### Phase 1: Scroll & Input Foundation (P0)

#### 1.1 Momentum Scrolling (`zedra-scroll`)

**Goal**: Make scrolling feel native with iOS-style deceleration.

**Current Problem**:
- Touch drag converts to `ScrollWheel` with raw delta
- No velocity tracking
- No momentum after touch ends
- No over-scroll bounce

**Solution**: Create `zedra-scroll` crate with:

```rust
// Core types
pub struct ScrollState {
    offset: Point,
    velocity: Vector2,
    is_tracking: bool,
    momentum_animator: MomentumAnimator,
}

// Deceleration curves (from zedra-gesture/velocity.rs)
pub enum DecelerationRate {
    Normal,  // 0.998 - iOS default
    Fast,    // 0.99  - Quick stop
    Custom(f32),
}

// ScrollView element wrapper
pub fn scroll_view() -> ScrollViewElement {
    ScrollViewElement::new()
}

impl ScrollViewElement {
    pub fn on_scroll(self, handler: impl Fn(ScrollEvent)) -> Self;
    pub fn deceleration_rate(self, rate: DecelerationRate) -> Self;
    pub fn bounces(self, bounces: bool) -> Self;
    pub fn shows_indicators(self, show: bool) -> Self;
    pub fn content_inset(self, inset: Edges<Pixels>) -> Self;
    pub fn child(self, child: impl IntoElement) -> Self;
}
```

**Implementation Steps**:
1. Move `VelocityTracker` and `MomentumAnimator` from `zedra-gesture` to shared util
2. Create `ScrollViewElement` that wraps child with momentum physics
3. Track touch velocity during drag
4. On touch end, start momentum animation
5. Apply deceleration curve each frame until velocity < threshold
6. Optional: Add over-scroll bounce effect

**Files to Create**:
- `crates/zedra-scroll/src/lib.rs`
- `crates/zedra-scroll/src/scroll_view.rs`
- `crates/zedra-scroll/src/momentum.rs`
- `crates/zedra-scroll/src/indicators.rs`

#### 1.2 TextInput Component (`zedra-input`)

**Goal**: Reusable text input with automatic keyboard management.

**Current Problem**:
- Must manually implement text fields in each view
- Keyboard show/hide requires explicit JNI calls
- No cursor positioning or selection
- No IME composition support

**Solution**: Create `zedra-input` crate:

```rust
pub fn text_input(state: Model<TextInputState>) -> TextInputElement {
    TextInputElement::new(state)
}

pub struct TextInputState {
    pub text: String,
    pub cursor_position: usize,
    pub selection: Option<Range<usize>>,
    pub is_focused: bool,
    pub placeholder: String,
    pub is_secure: bool,
}

impl TextInputElement {
    pub fn placeholder(self, text: &str) -> Self;
    pub fn secure(self, is_secure: bool) -> Self;
    pub fn keyboard_type(self, kt: KeyboardType) -> Self;
    pub fn return_key_type(self, rkt: ReturnKeyType) -> Self;
    pub fn on_change(self, handler: impl Fn(&str)) -> Self;
    pub fn on_submit(self, handler: impl Fn(&str)) -> Self;
    pub fn on_focus(self, handler: impl Fn(bool)) -> Self;
    pub fn auto_focus(self) -> Self;
}

pub enum KeyboardType {
    Default,
    EmailAddress,
    NumberPad,
    PhonePad,
    URL,
}

pub enum ReturnKeyType {
    Default,
    Go,
    Next,
    Search,
    Send,
    Done,
}
```

**Implementation Steps**:
1. Create `TextInputState` model for text, cursor, selection
2. Implement focus management with automatic keyboard show/hide
3. Handle `KeyDownEvent` for character input and control keys
4. Render text with cursor indicator
5. Add placeholder text when empty
6. Support secure entry (password dots)
7. Emit events: `on_change`, `on_submit`, `on_focus`

**Files to Create**:
- `crates/zedra-input/src/lib.rs`
- `crates/zedra-input/src/text_input.rs`
- `crates/zedra-input/src/keyboard.rs`
- `crates/zedra-input/src/cursor.rs`

---

### Phase 2: Animation System (P1)

#### 2.1 Shared Values & Animated Styles

**Goal**: React Native Reanimated-style gesture-driven animations.

**Inspiration** (Reanimated):
```javascript
const offset = useSharedValue(0);
const animatedStyle = useAnimatedStyle(() => ({
  transform: [{ translateY: offset.value }],
}));
```

**GPUI Equivalent**:
```rust
// Shared value - can be updated from gesture handlers
let offset = cx.new_model(|_| AnimatedValue::new(0.0));

// Animated style - recomputes when value changes
div()
    .animated_style(offset.clone(), |value| {
        Style::default().transform(Transform::translate_y(px(value)))
    })
    .child(content)
```

**Core Types**:
```rust
pub struct AnimatedValue<T: Animatable> {
    current: T,
    target: Option<T>,
    animation: Option<Animation>,
}

impl<T: Animatable> AnimatedValue<T> {
    pub fn set(&mut self, value: T);
    pub fn animate_to(&mut self, value: T, animation: Animation);
    pub fn spring_to(&mut self, value: T, spring: SpringConfig);
}

pub trait Animatable: Clone + 'static {
    fn interpolate(&self, to: &Self, progress: f32) -> Self;
}

// Implement for common types
impl Animatable for f32 { ... }
impl Animatable for Pixels { ... }
impl Animatable for Hsla { ... }
impl Animatable for Point { ... }
```

#### 2.2 Spring Physics

```rust
pub struct SpringConfig {
    pub damping: f32,      // 10-20 typical
    pub stiffness: f32,    // 100-300 typical
    pub mass: f32,         // Usually 1.0
}

impl SpringConfig {
    pub fn default() -> Self;      // Balanced feel
    pub fn bouncy() -> Self;       // More oscillation
    pub fn stiff() -> Self;        // Quick, minimal bounce
    pub fn gentle() -> Self;       // Slow, smooth
}
```

#### 2.3 Gesture-Animation Integration

```rust
// Example: Swipeable drawer with spring animation
let drawer_offset = cx.new_model(|_| AnimatedValue::new(0.0));

pan_gesture()
    .on_change({
        let offset = drawer_offset.clone();
        move |event, _window, cx| {
            offset.update(cx, |v, _| v.set(event.translation.x));
        }
    })
    .on_end({
        let offset = drawer_offset.clone();
        move |event, _window, cx| {
            let target = if event.translation.x > 100.0 { 280.0 } else { 0.0 };
            offset.update(cx, |v, _| v.spring_to(target, SpringConfig::default()));
        }
    })
    .child(
        div()
            .animated_style(drawer_offset, |x| {
                Style::default().left(px(x - 280.0))
            })
            .child(drawer_content)
    )
```

---

### Phase 3: Safe Area & Platform Integration (P1)

#### 3.1 Safe Area Insets

**Goal**: Properly handle notches, home indicators, and system UI.

```rust
// Get safe area insets (from Android WindowInsets)
pub struct SafeAreaInsets {
    pub top: Pixels,      // Status bar + notch
    pub bottom: Pixels,   // Home indicator / nav bar
    pub left: Pixels,     // Side notch (foldables)
    pub right: Pixels,
}

// Provider component
pub fn safe_area_provider() -> SafeAreaProvider {
    SafeAreaProvider::new()
}

// Hook to get insets
pub fn use_safe_area_insets(cx: &mut Context) -> SafeAreaInsets {
    cx.global::<SafeAreaInsets>().clone()
}

// Convenience wrapper
pub fn safe_area_view() -> SafeAreaView {
    SafeAreaView::new()
}

impl SafeAreaView {
    pub fn edges(self, edges: Edges) -> Self;  // Which edges to apply
    pub fn child(self, child: impl IntoElement) -> Self;
}
```

**Android Integration**:
```java
// GpuiSurfaceView.java
ViewCompat.setOnApplyWindowInsetsListener(this, (v, insets) -> {
    Insets systemBars = insets.getInsets(WindowInsetsCompat.Type.systemBars());
    nativeSetSafeAreaInsets(systemBars.top, systemBars.bottom,
                            systemBars.left, systemBars.right);
    return insets;
});
```

#### 3.2 Keyboard Avoidance

```rust
// Automatically adjust layout when keyboard appears
pub fn keyboard_avoiding_view() -> KeyboardAvoidingView {
    KeyboardAvoidingView::new()
}

impl KeyboardAvoidingView {
    pub fn behavior(self, behavior: KeyboardBehavior) -> Self;
    pub fn child(self, child: impl IntoElement) -> Self;
}

pub enum KeyboardBehavior {
    Height,   // Reduce height
    Position, // Move up
    Padding,  // Add bottom padding
}
```

---

### Phase 4: Navigation Enhancements (P2)

#### 4.1 Transition Animations

```rust
impl StackNavigator {
    pub fn push_animated(&mut self, screen: AnyView, transition: Transition) {
        // Animate incoming screen from right
        // Animate outgoing screen to left (parallax)
    }

    pub fn pop_animated(&mut self, transition: Transition) {
        // Reverse of push
    }
}

pub enum Transition {
    SlideFromRight,    // iOS default
    SlideFromBottom,   // Modal style
    Fade,
    None,
    Custom(Box<dyn TransitionAnimator>),
}
```

#### 4.2 Gesture-Based Navigation

```rust
// Swipe from left edge to pop
impl StackNavigator {
    pub fn enable_swipe_back(self, enabled: bool) -> Self;
    pub fn swipe_back_threshold(self, threshold: Pixels) -> Self;
}

// Swipe to close drawer
impl DrawerHost {
    pub fn enable_swipe_gesture(self, enabled: bool) -> Self;
}
```

#### 4.3 Deep Linking

```rust
pub struct NavigationLink {
    pub path: String,
    pub params: HashMap<String, String>,
}

impl StackNavigator {
    pub fn navigate_to(&mut self, link: &str);  // "app://settings/profile"
    pub fn register_route(&mut self, pattern: &str, builder: ScreenBuilder);
}
```

---

### Phase 5: List Enhancements (P2)

#### 5.1 Swipe Actions

```rust
pub fn swipeable_row() -> SwipeableRow {
    SwipeableRow::new()
}

impl SwipeableRow {
    pub fn left_actions(self, actions: Vec<SwipeAction>) -> Self;
    pub fn right_actions(self, actions: Vec<SwipeAction>) -> Self;
    pub fn child(self, child: impl IntoElement) -> Self;
}

pub struct SwipeAction {
    pub label: String,
    pub color: Hsla,
    pub icon: Option<Icon>,
    pub on_press: Box<dyn Fn()>,
}
```

#### 5.2 Pull to Refresh

```rust
impl ScrollViewElement {
    pub fn refresh_control(self, control: RefreshControl) -> Self;
}

pub struct RefreshControl {
    pub is_refreshing: bool,
    pub on_refresh: Box<dyn Fn()>,
    pub colors: Vec<Hsla>,
}
```

#### 5.3 Section Lists

```rust
pub fn section_list<S, I>() -> SectionList<S, I> {
    SectionList::new()
}

impl<S, I> SectionList<S, I> {
    pub fn sections(self, sections: Vec<Section<S, I>>) -> Self;
    pub fn render_header(self, renderer: impl Fn(&S) -> AnyElement) -> Self;
    pub fn render_item(self, renderer: impl Fn(&I) -> AnyElement) -> Self;
    pub fn sticky_headers(self, sticky: bool) -> Self;
}
```

---

## Crate Structure

```
crates/
├── zedra-core/           # Shared utilities & platform abstractions
│   ├── lib.rs
│   ├── animation.rs      # AnimatedValue, Spring, Easing
│   ├── platform.rs       # SafeAreaInsets, KeyboardState, ColorScheme
│   ├── storage.rs        # Storage, SecureStorage, FileStorage
│   ├── haptics.rs        # HapticFeedback
│   ├── clipboard.rs      # Clipboard
│   └── types.rs          # Common types
│
├── zedra-assets/         # Asset loading & management
│   ├── lib.rs
│   ├── loader.rs         # AssetLoader, AssetSource
│   ├── image.rs          # ImageAsset, ImageElement
│   ├── cache.rs          # Asset caching
│   └── bundled.rs        # Compile-time asset embedding
│
├── zedra-fonts/          # Font system
│   ├── lib.rs
│   ├── registry.rs       # FontRegistry, FontFamily
│   ├── loader.rs         # Font loading from assets
│   └── fallback.rs       # Fallback chain management
│
├── zedra-icons/          # Icon system
│   ├── lib.rs
│   ├── registry.rs       # IconRegistry
│   ├── element.rs        # IconElement
│   └── sets/             # Built-in icon sets
│       ├── material.rs
│       └── feather.rs
│
├── zedra-theme/          # Theming system
│   ├── lib.rs
│   ├── theme.rs          # Theme, ThemeColors, ThemeTypography
│   ├── provider.rs       # ThemeProvider
│   ├── presets/          # Built-in themes
│   │   ├── light.rs
│   │   └── dark.rs
│   └── components/       # Pre-styled components
│       ├── button.rs
│       ├── card.rs
│       └── chip.rs
│
├── zedra-scroll/         # Scrolling primitives
│   ├── lib.rs
│   ├── scroll_view.rs    # Momentum scrolling
│   ├── momentum.rs       # Physics calculations
│   ├── indicators.rs     # Scroll indicators
│   ├── refresh.rs        # Pull to refresh
│   └── pager.rs          # Horizontal paging
│
├── zedra-input/          # Input components
│   ├── lib.rs
│   ├── text_input.rs     # TextInput element
│   ├── text_area.rs      # Multi-line input
│   ├── keyboard.rs       # Keyboard management & avoidance
│   ├── picker.rs         # Selection picker
│   ├── slider.rs         # Slider input
│   ├── switch.rs         # Toggle switch
│   └── checkbox.rs       # Checkbox
│
├── zedra-gesture/        # Gesture recognition (exists)
│   ├── lib.rs
│   ├── element.rs        # Gesture wrapper elements
│   ├── compose.rs        # race, simultaneous, exclusive
│   ├── compositor.rs     # Gesture arbitration
│   ├── types.rs          # GestureId, TouchEvent, etc.
│   ├── velocity.rs       # VelocityTracker, MomentumAnimator
│   ├── state.rs          # GestureState
│   └── recognizers/
│       ├── mod.rs
│       ├── tap.rs
│       ├── pan.rs
│       ├── pinch.rs
│       ├── long_press.rs
│       └── fling.rs
│
├── zedra-nav/            # Navigation (exists)
│   ├── lib.rs
│   ├── stack.rs          # StackNavigator
│   ├── tab.rs            # TabNavigator
│   ├── drawer.rs         # DrawerHost
│   ├── modal.rs          # ModalHost
│   ├── transitions.rs    # Transition animations
│   └── deep_link.rs      # URL-based navigation
│
├── zedra-list/           # List components
│   ├── lib.rs
│   ├── swipeable.rs      # Swipe actions
│   ├── section_list.rs   # Grouped lists with headers
│   ├── reorderable.rs    # Drag to reorder
│   └── infinite.rs       # Infinite scroll helper
│
├── zedra-http/           # HTTP client
│   ├── lib.rs
│   ├── client.rs         # HttpClient
│   ├── request.rs        # RequestBuilder
│   └── response.rs       # Response handling
│
├── zedra-ui/             # Meta-crate re-exporting all UI components
│   └── lib.rs            # pub use zedra_*
│
└── zedra/                # Android cdylib (exists)
    ├── lib.rs            # JNI exports
    ├── android_app.rs    # Main app
    ├── android_jni.rs    # JNI bridge
    ├── android_command_queue.rs
    └── platform/         # Platform-specific implementations
        ├── mod.rs
        ├── safe_area.rs  # WindowInsets → SafeAreaInsets
        ├── keyboard.rs   # IME management
        ├── storage.rs    # SharedPreferences binding
        ├── haptics.rs    # Vibrator binding
        ├── assets.rs     # AssetManager binding
        └── permissions.rs # Runtime permissions
```

### Dependency Graph

```
                    ┌─────────────┐
                    │  zedra-ui   │ (meta-crate)
                    └──────┬──────┘
                           │
        ┌──────────────────┼──────────────────┐
        │                  │                  │
        ▼                  ▼                  ▼
┌───────────────┐  ┌───────────────┐  ┌───────────────┐
│  zedra-nav    │  │ zedra-input   │  │  zedra-list   │
└───────┬───────┘  └───────┬───────┘  └───────┬───────┘
        │                  │                  │
        └────────┬─────────┴─────────┬────────┘
                 │                   │
                 ▼                   ▼
         ┌───────────────┐   ┌───────────────┐
         │ zedra-scroll  │   │ zedra-gesture │
         └───────┬───────┘   └───────┬───────┘
                 │                   │
                 └─────────┬─────────┘
                           │
                           ▼
                   ┌───────────────┐
                   │  zedra-core   │
                   └───────┬───────┘
                           │
        ┌──────────────────┼──────────────────┐
        │                  │                  │
        ▼                  ▼                  ▼
┌───────────────┐  ┌───────────────┐  ┌───────────────┐
│ zedra-assets  │  │  zedra-fonts  │  │  zedra-theme  │
└───────────────┘  └───────────────┘  └───────────────┘
                           │
                           ▼
                   ┌───────────────┐
                   │     gpui      │
                   └───────────────┘
```

---

---

### Phase 6: Assets & Resources (P1)

#### 6.1 Font System (`zedra-fonts`)

**Current State**:
- GPUI uses CosmicTextSystem on Android
- Fonts loaded from system paths (`/system/fonts/`)
- No custom font loading
- No font weight/style variants

**Goal**: Easy custom font loading with fallback chains.

```rust
// Font family registration
pub struct FontFamily {
    pub name: String,
    pub weights: HashMap<FontWeight, FontSource>,
}

pub enum FontSource {
    Asset(String),           // "fonts/Inter-Regular.ttf"
    System(String),          // "Roboto"
    Bytes(&'static [u8]),    // Embedded font
}

// Font registry (global)
pub struct FontRegistry {
    families: HashMap<String, FontFamily>,
    fallback_chain: Vec<String>,
}

impl FontRegistry {
    pub fn register(&mut self, family: FontFamily);
    pub fn set_default(&mut self, family: &str);
    pub fn set_fallback_chain(&mut self, chain: Vec<&str>);
}

// Usage in app initialization
fn init_fonts(cx: &mut App) {
    let fonts = cx.global_mut::<FontRegistry>();

    fonts.register(FontFamily {
        name: "Inter".into(),
        weights: hashmap! {
            FontWeight::NORMAL => FontSource::Asset("fonts/Inter-Regular.ttf"),
            FontWeight::MEDIUM => FontSource::Asset("fonts/Inter-Medium.ttf"),
            FontWeight::BOLD => FontSource::Asset("fonts/Inter-Bold.ttf"),
        },
    });

    fonts.set_default("Inter");
    fonts.set_fallback_chain(vec!["Inter", "Roboto", "sans-serif"]);
}

// Usage in elements
div()
    .font_family("Inter")
    .font_weight(FontWeight::MEDIUM)
    .child("Hello")
```

**Android Integration**:
```rust
// Load font from APK assets
pub fn load_font_from_assets(path: &str) -> Result<Vec<u8>> {
    // JNI call to AssetManager.open()
    android_load_asset(path)
}
```

#### 6.2 Asset Loader (`zedra-assets`)

**Goal**: Unified asset loading for images, fonts, JSON, and other resources.

```rust
pub struct AssetLoader {
    cache: HashMap<String, AssetHandle>,
    base_path: PathBuf,
}

pub enum Asset {
    Image(ImageAsset),
    Font(FontAsset),
    Json(serde_json::Value),
    Text(String),
    Binary(Vec<u8>),
}

pub struct ImageAsset {
    pub data: Arc<ImageData>,
    pub size: Size<Pixels>,
    pub format: ImageFormat,
}

impl AssetLoader {
    /// Load asset synchronously (blocking)
    pub fn load(&mut self, path: &str) -> Result<Asset>;

    /// Load asset asynchronously
    pub fn load_async(&self, path: &str) -> Task<Result<Asset>>;

    /// Preload assets for faster access
    pub fn preload(&mut self, paths: &[&str]) -> Task<()>;

    /// Clear cache
    pub fn clear_cache(&mut self);

    /// Get memory usage
    pub fn cache_size(&self) -> usize;
}

// Platform-specific asset sources
pub enum AssetSource {
    Bundled,           // APK assets folder
    FileSystem(PathBuf),
    Remote(Url),
    Memory(&'static [u8]),
}

// Usage
let loader = cx.global::<AssetLoader>();
let icon = loader.load("images/icon.png")?;
```

#### 6.3 Image Component

**Goal**: Async image loading with placeholder and error states.

```rust
pub fn image(source: ImageSource) -> ImageElement {
    ImageElement::new(source)
}

pub enum ImageSource {
    Asset(String),           // "images/logo.png"
    Url(String),             // "https://..."
    Data(Arc<ImageData>),    // Pre-loaded data
    Placeholder,             // Empty placeholder
}

impl ImageElement {
    pub fn placeholder(self, element: impl IntoElement) -> Self;
    pub fn error(self, element: impl IntoElement) -> Self;
    pub fn loading(self, element: impl IntoElement) -> Self;
    pub fn resize_mode(self, mode: ResizeMode) -> Self;
    pub fn fade_in(self, duration: Duration) -> Self;
    pub fn on_load(self, handler: impl Fn(Size<Pixels>)) -> Self;
    pub fn on_error(self, handler: impl Fn(ImageError)) -> Self;
}

pub enum ResizeMode {
    Cover,      // Fill, crop excess
    Contain,    // Fit, letterbox
    Stretch,    // Distort to fill
    Center,     // Original size, centered
}

// Usage
image(ImageSource::Url("https://example.com/photo.jpg"))
    .resize_mode(ResizeMode::Cover)
    .placeholder(div().bg(gray()).size_full())
    .error(div().child("Failed to load"))
    .fade_in(Duration::from_millis(200))
```

#### 6.4 Icon System

**Goal**: Vector icons with easy theming.

```rust
// Icon registry (SF Symbols / Material Icons style)
pub struct IconRegistry {
    icons: HashMap<String, IconData>,
}

pub enum IconData {
    Svg(String),
    Path(Vec<PathCommand>),
    Font { family: String, codepoint: char },
}

// Icon element
pub fn icon(name: &str) -> IconElement {
    IconElement::new(name)
}

impl IconElement {
    pub fn size(self, size: Pixels) -> Self;
    pub fn color(self, color: impl Into<Hsla>) -> Self;
    pub fn weight(self, weight: IconWeight) -> Self;  // For variable fonts
}

pub enum IconWeight {
    Light,
    Regular,
    Medium,
    Bold,
}

// Usage
icon("chevron-left")
    .size(px(24.0))
    .color(rgb(0x007AFF))
```

---

### Phase 7: Theming & Styling (P1)

#### 7.1 Theme System

**Goal**: Consistent theming with light/dark mode support.

```rust
pub struct Theme {
    pub colors: ThemeColors,
    pub typography: ThemeTypography,
    pub spacing: ThemeSpacing,
    pub radii: ThemeRadii,
    pub shadows: ThemeShadows,
}

pub struct ThemeColors {
    // Semantic colors
    pub primary: Hsla,
    pub secondary: Hsla,
    pub accent: Hsla,
    pub background: Hsla,
    pub surface: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub success: Hsla,

    // Text colors
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_disabled: Hsla,
    pub text_inverse: Hsla,

    // Border colors
    pub border: Hsla,
    pub border_focused: Hsla,

    // System
    pub separator: Hsla,
    pub overlay: Hsla,
}

pub struct ThemeTypography {
    pub font_family: String,
    pub font_family_mono: String,

    pub size_xs: Pixels,    // 11
    pub size_sm: Pixels,    // 13
    pub size_md: Pixels,    // 15
    pub size_lg: Pixels,    // 17
    pub size_xl: Pixels,    // 20
    pub size_2xl: Pixels,   // 24
    pub size_3xl: Pixels,   // 30

    pub line_height_tight: f32,   // 1.2
    pub line_height_normal: f32,  // 1.5
    pub line_height_loose: f32,   // 1.8
}

// Theme provider
pub fn theme_provider(theme: Theme) -> ThemeProvider {
    ThemeProvider::new(theme)
}

// Access theme in components
pub fn use_theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

// Convenience accessors
pub fn use_colors(cx: &App) -> &ThemeColors {
    &use_theme(cx).colors
}
```

#### 7.2 Color Scheme Detection

```rust
pub enum ColorScheme {
    Light,
    Dark,
    System,  // Follow system setting
}

// Get current system color scheme
pub fn use_color_scheme(cx: &App) -> ColorScheme {
    cx.global::<SystemColorScheme>().current
}

// React to system changes
impl App {
    pub fn on_color_scheme_change(&mut self, handler: impl Fn(ColorScheme));
}

// Usage
let colors = if use_color_scheme(cx) == ColorScheme::Dark {
    ThemeColors::dark()
} else {
    ThemeColors::light()
};
```

#### 7.3 Styled Components

```rust
// Pre-styled component variants
pub fn button(label: &str) -> ButtonElement {
    ButtonElement::new(label)
}

impl ButtonElement {
    pub fn variant(self, variant: ButtonVariant) -> Self;
    pub fn size(self, size: ButtonSize) -> Self;
    pub fn disabled(self, disabled: bool) -> Self;
    pub fn loading(self, loading: bool) -> Self;
    pub fn on_press(self, handler: impl Fn()) -> Self;
}

pub enum ButtonVariant {
    Primary,
    Secondary,
    Outline,
    Ghost,
    Destructive,
}

pub enum ButtonSize {
    Small,   // 32px height
    Medium,  // 40px height
    Large,   // 48px height
}

// Usage
button("Submit")
    .variant(ButtonVariant::Primary)
    .size(ButtonSize::Medium)
    .on_press(|| submit_form())
```

---

### Phase 8: Platform APIs (P2)

#### 8.1 Storage

```rust
// Key-value storage (SharedPreferences / UserDefaults)
pub struct Storage {
    // ...
}

impl Storage {
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T>;
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T);
    pub fn remove(&mut self, key: &str);
    pub fn clear(&mut self);
}

// Secure storage (Keychain / Keystore)
pub struct SecureStorage {
    // ...
}

impl SecureStorage {
    pub fn get(&self, key: &str) -> Option<String>;
    pub fn set(&mut self, key: &str, value: &str) -> Result<()>;
    pub fn remove(&mut self, key: &str);
}

// File storage
pub struct FileStorage {
    // ...
}

impl FileStorage {
    pub fn documents_dir(&self) -> PathBuf;
    pub fn cache_dir(&self) -> PathBuf;
    pub fn read(&self, path: &Path) -> Result<Vec<u8>>;
    pub fn write(&self, path: &Path, data: &[u8]) -> Result<()>;
    pub fn delete(&self, path: &Path) -> Result<()>;
}
```

#### 8.2 Networking

```rust
// HTTP client
pub struct HttpClient {
    // ...
}

impl HttpClient {
    pub fn get(&self, url: &str) -> RequestBuilder;
    pub fn post(&self, url: &str) -> RequestBuilder;
    pub fn put(&self, url: &str) -> RequestBuilder;
    pub fn delete(&self, url: &str) -> RequestBuilder;
}

impl RequestBuilder {
    pub fn header(self, key: &str, value: &str) -> Self;
    pub fn json<T: Serialize>(self, body: &T) -> Self;
    pub fn timeout(self, duration: Duration) -> Self;
    pub fn send(self) -> Task<Result<Response>>;
}

// Usage
let response = http.get("https://api.example.com/users")
    .header("Authorization", "Bearer token")
    .timeout(Duration::from_secs(30))
    .send()
    .await?;

let users: Vec<User> = response.json()?;
```

#### 8.3 Permissions

```rust
pub enum Permission {
    Camera,
    Microphone,
    PhotoLibrary,
    Location,
    LocationAlways,
    Notifications,
    Contacts,
    Calendar,
}

pub enum PermissionStatus {
    NotDetermined,
    Denied,
    Authorized,
    Restricted,  // iOS parental controls
}

pub struct Permissions {
    // ...
}

impl Permissions {
    pub fn status(&self, permission: Permission) -> PermissionStatus;
    pub fn request(&self, permission: Permission) -> Task<PermissionStatus>;
    pub fn open_settings(&self);
}
```

#### 8.4 Haptics

```rust
pub enum HapticFeedback {
    Light,
    Medium,
    Heavy,
    Selection,
    Success,
    Warning,
    Error,
}

pub fn haptic(feedback: HapticFeedback) {
    // Trigger haptic feedback
}

// Usage in gesture handler
tap_gesture()
    .on_tap(|_, _, _| {
        haptic(HapticFeedback::Light);
    })
```

#### 8.5 Clipboard

```rust
pub struct Clipboard {
    // ...
}

impl Clipboard {
    pub fn get_text(&self) -> Option<String>;
    pub fn set_text(&mut self, text: &str);
    pub fn has_text(&self) -> bool;
    pub fn clear(&mut self);
}
```

---

### Phase 9: Developer Experience (P2)

#### 9.1 Hot Reload (Future)

**Goal**: Instant UI updates without full rebuild.

```rust
// Development mode with live reload
#[cfg(debug_assertions)]
pub fn enable_hot_reload(cx: &mut App) {
    // Watch for file changes
    // Re-render affected components
}
```

#### 9.2 Debug Tools

```rust
// Layout inspector
pub fn enable_layout_inspector(cx: &mut App) {
    // Show element bounds on long-press
    // Display element hierarchy
}

// Performance overlay
pub fn enable_perf_overlay(cx: &mut App) {
    // Show FPS counter
    // Show memory usage
    // Show render times
}

// Network inspector
pub fn enable_network_inspector(cx: &mut App) {
    // Log all HTTP requests
    // Show request/response details
}
```

#### 9.3 Sandbox / Preview System (`zedra-sandbox`)

**Goal**: SwiftUI Preview / Storybook-like component isolation and testing.

**Inspiration**:
- **SwiftUI Preview**: Live preview in Xcode, multiple device sizes
- **Jetpack Compose Preview**: `@Preview` annotation, interactive mode
- **Storybook**: Component catalog, props playground, documentation

```rust
// Define a preview for a component
#[preview]
fn button_preview() -> impl IntoElement {
    Preview::new("Button")
        .with_variant("Primary", || {
            button("Submit").variant(ButtonVariant::Primary)
        })
        .with_variant("Secondary", || {
            button("Cancel").variant(ButtonVariant::Secondary)
        })
        .with_variant("Disabled", || {
            button("Disabled").disabled(true)
        })
}

// Preview with different states
#[preview]
fn text_input_preview() -> impl IntoElement {
    Preview::new("TextInput")
        .with_state("Empty", TextInputState::default())
        .with_state("Filled", TextInputState { text: "Hello".into(), ..default() })
        .with_state("Error", TextInputState { error: Some("Invalid".into()), ..default() })
        .render(|state| text_input(state))
}

// Preview with device frames
#[preview]
fn login_screen_preview() -> impl IntoElement {
    Preview::new("LoginScreen")
        .device(Device::IPhone14)
        .dark_mode(true)
        .render(|| LoginScreen::new())
}
```

**Sandbox App Features**:

```rust
// Main sandbox app that collects all previews
fn main() {
    App::new().run(|cx| {
        let sandbox = Sandbox::new()
            // Auto-discover previews from crates
            .discover_previews()
            // Or manually register
            .register(button_preview)
            .register(text_input_preview)
            .register(login_screen_preview);

        cx.open_window(sandbox);
    });
}
```

**Sandbox UI**:
```
┌─────────────────────────────────────────────────────────────┐
│  🔍 Search components...                              ☀️ 🌙 │
├─────────────────────────────────────────────────────────────┤
│ ┌───────────┐ ┌─────────────────────────────────────────┐   │
│ │ Components│ │                                         │   │
│ ├───────────┤ │         ┌─────────────────┐             │   │
│ │ ▼ Inputs  │ │         │                 │             │   │
│ │   Button  │ │         │   [  Submit  ]  │             │   │
│ │   TextIn..│ │         │                 │             │   │
│ │   Switch  │ │         └─────────────────┘             │   │
│ │ ▼ Layout  │ │                                         │   │
│ │   Card    │ │  Variant: ○ Primary ● Secondary ○ Ghost │   │
│ │   Stack   │ │  Size:    ○ Small  ● Medium  ○ Large    │   │
│ │ ▼ Screens │ │  Disabled: [ ]                          │   │
│ │   Login   │ │                                         │   │
│ │   Home    │ │  Device: [iPhone 14 ▼]  Dark: [✓]       │   │
│ └───────────┘ └─────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Implementation**:

```rust
// crates/zedra-sandbox/src/lib.rs

pub struct Sandbox {
    previews: Vec<PreviewEntry>,
    selected: Option<usize>,
    search: String,
    dark_mode: bool,
    device: Device,
}

pub struct PreviewEntry {
    pub name: String,
    pub category: String,
    pub variants: Vec<PreviewVariant>,
    pub render: Box<dyn Fn() -> AnyElement>,
}

pub struct PreviewVariant {
    pub name: String,
    pub props: HashMap<String, PropValue>,
}

pub enum PropValue {
    Bool(bool),
    String(String),
    Number(f64),
    Enum(String, Vec<String>),
    Color(Hsla),
}

pub struct Preview {
    name: String,
    category: String,
    variants: Vec<PreviewVariant>,
    device: Option<Device>,
    dark_mode: bool,
}

impl Preview {
    pub fn new(name: &str) -> Self;
    pub fn category(self, category: &str) -> Self;
    pub fn with_variant(self, name: &str, render: impl Fn() -> AnyElement) -> Self;
    pub fn with_prop<T: IntoPropValue>(self, name: &str, default: T) -> Self;
    pub fn device(self, device: Device) -> Self;
    pub fn dark_mode(self, enabled: bool) -> Self;
    pub fn render(self, render: impl Fn() -> AnyElement) -> PreviewEntry;
}

#[derive(Clone, Copy)]
pub enum Device {
    IPhone14,
    IPhone14Pro,
    IPhone14ProMax,
    IPhoneSE,
    Pixel7,
    Pixel7Pro,
    GalaxyS23,
    IPadMini,
    IPadPro11,
    Custom { width: u32, height: u32, scale: f32 },
}

impl Device {
    pub fn size(&self) -> Size<Pixels>;
    pub fn scale(&self) -> f32;
    pub fn safe_area(&self) -> SafeAreaInsets;
}
```

**CLI for Sandbox**:
```bash
# Run sandbox app
cargo run --package zedra-sandbox

# Run with specific component focused
cargo run --package zedra-sandbox -- --component Button

# Run in dark mode
cargo run --package zedra-sandbox -- --dark

# Export component screenshots
cargo run --package zedra-sandbox -- --export ./screenshots
```

**Integration with Main App**:
```rust
// In development, add sandbox toggle
#[cfg(debug_assertions)]
fn dev_menu(cx: &mut Context<Self>) -> impl IntoElement {
    div()
        .absolute()
        .bottom_4()
        .right_4()
        .child(
            button("🧪")
                .on_press(|| open_sandbox(cx))
        )
}
```

#### 9.4 Accessibility

```rust
impl Element {
    pub fn accessible_label(self, label: &str) -> Self;
    pub fn accessible_hint(self, hint: &str) -> Self;
    pub fn accessible_role(self, role: AccessibilityRole) -> Self;
    pub fn accessible_value(self, value: &str) -> Self;
    pub fn accessible_hidden(self, hidden: bool) -> Self;
}

pub enum AccessibilityRole {
    Button,
    Link,
    Header,
    Image,
    TextField,
    StaticText,
    SearchField,
    TabBar,
    Tab,
}
```

---

## Priority Matrix

| Feature | Effort | Impact | Priority |
|---------|--------|--------|----------|
| Momentum Scrolling | Medium | High | **P0** |
| TextInput Component | Medium | High | **P0** |
| Gesture-Driven Animations | High | High | **P1** |
| Safe Area Insets | Low | Medium | **P1** |
| Keyboard Avoidance | Medium | Medium | **P1** |
| Font System | Medium | Medium | **P1** |
| Asset Loader | Medium | Medium | **P1** |
| Image Component | Medium | Medium | **P1** |
| Theme System | Medium | Medium | **P1** |
| Navigation Transitions | Medium | Medium | **P2** |
| Swipe-to-Pop | Low | Medium | **P2** |
| Pull to Refresh | Low | Low | **P2** |
| Storage APIs | Low | Medium | **P2** |
| HTTP Client | Low | Medium | **P2** |
| Icon System | Low | Low | **P2** |
| Haptics | Low | Low | **P2** |
| Swipe Actions | Medium | Low | **P3** |
| Section Lists | Low | Low | **P3** |
| Deep Linking | Medium | Low | **P3** |
| Permissions | Medium | Low | **P3** |
| Hot Reload | High | Medium | **Future** |
| Accessibility | Medium | Medium | **Future** |

---

## Success Metrics

### Phase 1 Complete When:
- [ ] ScrollView with momentum feels like native iOS/Android
- [ ] TextInput auto-shows keyboard on focus
- [ ] Tab cycling between inputs works
- [ ] Password masking works

### Phase 2 Complete When:
- [ ] Drawer slides open/closed with spring physics
- [ ] Gesture-interrupted animations continue smoothly
- [ ] 60 FPS maintained during animations

### Phase 3 Complete When:
- [ ] Content doesn't render under notch
- [ ] Keyboard doesn't cover focused input
- [ ] Safe area insets update on orientation change

### Phase 4 Complete When:
- [ ] Stack push/pop has slide animation
- [ ] Swipe from left edge pops screen
- [ ] Modal presents with slide-up animation

---

---

## React Native vs GPUI Mobile Comparison

| Aspect | React Native | GPUI Mobile | Advantage |
|--------|--------------|-------------|-----------|
| **Language** | JavaScript/TypeScript | Rust | GPUI: No GC pauses, predictable performance |
| **Threading** | JS Thread + Native Thread | Single-threaded | RN: Better for CPU-intensive UI work |
| **Gestures** | Native recognizers (RNGH) | Native via JNI | Equal: Both use platform gestures |
| **Animations** | UI thread (Reanimated) | Main thread | RN: Worklets are more flexible |
| **Bundle Size** | ~7MB minimum | ~3MB | GPUI: Smaller binaries |
| **Startup Time** | 300-500ms | ~50ms | GPUI: Much faster cold start |
| **Memory** | Higher (JS runtime) | Lower | GPUI: Better for low-end devices |
| **Hot Reload** | Excellent | Not yet | RN: Better DX currently |
| **Ecosystem** | Massive | Growing | RN: More libraries available |
| **Type Safety** | TypeScript (optional) | Rust (enforced) | GPUI: Compile-time guarantees |

### Why GPUI Mobile Can Succeed

1. **Performance**: Rust's zero-cost abstractions mean 60 FPS is easier to maintain
2. **Startup**: No JS engine initialization = instant app launch
3. **Memory**: No garbage collector = predictable memory usage
4. **Binary Size**: Smaller APKs = faster downloads
5. **Shared Code**: Same language for mobile UI and backend logic

### Challenges to Address

1. **Developer Experience**: Need better tooling (hot reload, debugging)
2. **Ecosystem**: Need more pre-built components
3. **Documentation**: Need comprehensive guides
4. **Community**: Need to grow contributor base

---

## Implementation Timeline (Suggested)

### Q1 2026: Foundation
- [ ] Momentum scrolling (zedra-scroll)
- [ ] TextInput component (zedra-input)
- [ ] Safe area insets
- [ ] Basic theme system

### Q2 2026: Polish
- [ ] Gesture-driven animations
- [ ] Navigation transitions
- [ ] Font system
- [ ] Asset loader
- [ ] Image component

### Q3 2026: Platform
- [ ] Storage APIs
- [ ] HTTP client
- [ ] Haptics
- [ ] Permissions

### Q4 2026: Ecosystem
- [ ] Icon library
- [ ] Component library
- [ ] Documentation site
- [ ] Example apps

---

## Next Steps

1. **Immediate**: Start with `zedra-scroll` for momentum scrolling
2. **This Week**: Implement basic `TextInput` component
3. **This Month**: Complete Phase 1 (Scroll + Input)
4. **Review**: Evaluate architecture after Phase 1

---

## References

- [React Navigation Documentation](https://reactnavigation.org/docs/getting-started)
- [React Native Gesture Handler](https://docs.swmansion.com/react-native-gesture-handler/)
- [React Native Reanimated](https://docs.swmansion.com/react-native-reanimated/)
- [iOS Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/)
- [Material Design 3](https://m3.material.io/)
- [Flutter Architecture](https://docs.flutter.dev/resources/architectural-overview)
- [Tauri Mobile](https://tauri.app/blog/tauri-mobile-alpha/)

---

**Last Updated**: 2026-02-09
