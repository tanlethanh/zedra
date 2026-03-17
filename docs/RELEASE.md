# Releasing zedra-host

## How to cut a release

1. Bump the version in `Cargo.toml` (workspace root):
   ```toml
   [workspace.package]
   version = "0.2.0"
   ```

2. Commit, tag, and push:
   ```bash
   git add Cargo.toml
   git commit -m "chore: release v0.2.0"
   git push origin main
   git tag v0.2.0
   git push origin v0.2.0
   ```

   > **Do not use `git push --tags`** — this repo has 400+ tags pulled from the
   > upstream zed remote. Pushing them all would fail with HTTP 400.

Pushing the tag triggers `.github/workflows/release.yml`, which:
- Builds `zedra` for macOS arm64/x86_64 and Linux x86_64/aarch64
- Packages each binary as `zedra-<target>.tar.gz` with a `.sha256` checksum
- Creates a GitHub Release with all artifacts and auto-generated notes

## How users install / update

**First install:**
```bash
curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh
```

**Specific version:**
```bash
curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh -s -- --version v0.2.0
```

**Update** — run the install script again; it overwrites the existing binary.

The script installs to `~/.local/bin/zedra` by default. Override with `--prefix /usr/local/bin` or `ZEDRA_PREFIX`.

## Verify a release locally

```bash
cargo build -p zedra-host --release
./target/release/zedra --help
```

---

# iOS Release Pipeline

Two options are provided:
- **Option A** — GitHub Actions + xcodebuild (full control, runs on every push)
- **Option B** — Xcode Cloud (Apple-managed signing, native TestFlight integration)

## Prerequisites (both options)

- Apple Developer Program membership (team ID `4R7EAZY462`)
- App record created in App Store Connect (`dev.zedra.app`)
- Bundle ID `dev.zedra.app` registered in Certificates, Identifiers & Profiles

---

## Option A: GitHub Actions

### How it works

```
push v* tag  (or manual dispatch)
  │
  ├─ checkout + submodules
  ├─ install Rust aarch64-apple-ios + cargo cache
  ├─ brew install xcodegen + gem install cocoapods
  ├─ write ASC API key to ~/.appstoreconnect/private_keys/
  ├─ ./scripts/build-ios.sh --device --release   → ZedraFFI.xcframework
  ├─ cd ios && xcodegen generate && pod install  → Zedra.xcworkspace
  ├─ xcodebuild archive  (generic/platform=iOS, automatic signing via API key)
  ├─ xcodebuild -exportArchive  (ios/ExportOptions.plist → Zedra.ipa)
  ├─ upload IPA as workflow artifact (30-day retention)
  └─ xcrun altool --upload-app  → TestFlight
```

Workflow file: `.github/workflows/ios-release.yml`

### One-time setup

#### 1. Create an App Store Connect API key

1. App Store Connect → **Users and Access** → **Integrations** → **App Store Connect API**
2. Click **+**, name it (e.g. "CI"), role **Developer** (or **App Manager** for full access)
3. Download the `.p8` file — **Apple shows it once, save it**
4. Note the **Key ID** (10 chars) and **Issuer ID** (UUID at the top of the page)

#### 2. Add GitHub secrets

**Settings → Secrets and variables → Actions → New repository secret**

| Secret name | Value |
|---|---|
| `ASC_KEY_P8` | Full contents of the downloaded `.p8` file |
| `ASC_KEY_ID` | 10-character Key ID (e.g. `ABC1234567`) |
| `ASC_ISSUER_ID` | Issuer UUID (e.g. `12345678-1234-...`) |

#### 3. Verify ExportOptions.plist

`ios/ExportOptions.plist` is committed to the repo. Confirm the team ID matches:

```xml
<key>teamID</key>
<string>4R7EAZY462</string>
```

### Triggering a release

```bash
# Tag and push — triggers the workflow automatically
git tag v1.0.0
git push origin v1.0.0
```

Or trigger manually from **Actions → iOS Release → Run workflow** and choose
`testflight` or `skip-upload`.

### Xcode version note

`macos-15` runners ship with Xcode 16.x. The project targets iOS 16.0
(matching `ios/project.yml`). When GitHub-hosted runners include Xcode 26,
update `IPHONEOS_DEPLOYMENT_TARGET` in the workflow and `ios/project.yml`.
For Xcode 26 / iOS 26 builds today, use **Option B**.

### Upload step note

`xcrun altool --upload-app` is deprecated since Xcode 14 but still functional.
To replace it: use Fastlane `pilot`, or call the App Store Connect REST API
directly with the same P8 key.

---

## Option B: Xcode Cloud

### How it works

```
App Store Connect workflow trigger (branch / tag / PR)
  │
  ├─ clone repo + submodules
  ├─ ci_post_clone.sh
  │    ├─ brew install xcodegen && cd ios && xcodegen generate
  │    └─ curl rustup | sh + rustup target add aarch64-apple-ios
  ├─ Xcode Cloud: pod install  (automatic, detects Podfile)
  ├─ ci_pre_xcodebuild.sh
  │    └─ ./scripts/build-ios.sh --device [--release]
  ├─ xcodebuild archive  (Apple manages code signing)
  └─ post-action: distribute to TestFlight internal group
```

Custom scripts: `.xcode/cloud/ci_post_clone.sh`, `.xcode/cloud/ci_pre_xcodebuild.sh`

### One-time setup in App Store Connect

1. **App Store Connect → Xcode Cloud → Get Started**
   (or Xcode → Product → Xcode Cloud → Create Workflow)
2. Connect the GitHub repository when prompted (grant access via GitHub OAuth)
3. **Create Workflow**:
   - Name: `Release`
   - Start condition: **Tag** matching `v*`
   - Environment: **Xcode 26** (or latest), **macOS latest**
   - Actions: **Archive** → Scheme `Zedra`, Configuration `Release`
   - Post-action: **TestFlight (Internal Testing)** → select your internal group
4. Save and run the workflow once to verify signing works

Xcode Cloud automatically handles certificate creation/rotation, provisioning
profile management, and dSYM upload. No secrets needed in the repository.

### Submodule access

`vendor/zed` is a private submodule. Grant Xcode Cloud access to it in
**App Store Connect → Xcode Cloud → Settings → Source Code Management →
Additional Repositories**.

---

## Comparison

| | GitHub Actions | Xcode Cloud |
|---|---|---|
| Code signing | ASC API key → automatic provisioning | Apple-managed, zero config |
| Xcode 26 support | Not yet on hosted runners | Yes |
| Rust cargo cache | `Swatinem/rust-cache` (warm builds fast) | None (cold each time) |
| Upload to TestFlight | `xcrun altool` (deprecated but works) | Built-in post-action |
| Secrets to manage | 3 (`ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`) | None |
| Free tier | GitHub Actions minutes (varies by plan) | 25 compute hours/month |
| Submodule access | Automatic via checkout token | Requires explicit grant |

---

## Version bumping (iOS)

Before tagging a release, update the version in `ios/project.yml`:

```yaml
settings:
  base:
    MARKETING_VERSION: "1.1.0"   # user-facing version (CFBundleShortVersionString)
    CURRENT_PROJECT_VERSION: "2" # build number (CFBundleVersion), must increment
```

Then regenerate the project and commit:

```bash
cd ios && xcodegen generate
git add ios/Zedra.xcodeproj ios/project.yml
git commit -m "chore(ios): bump version to 1.1.0 (build 2)"
git tag v1.1.0
git push origin main --tags
```

`CURRENT_PROJECT_VERSION` must be strictly greater than the previous build number
accepted by App Store Connect — it does not need to match the tag number.

---

## Manual Release (local Mac)

Use this when CI is unavailable or you want to ship a hotfix without waiting for
the automated pipeline. Requires Xcode signed in with an Apple ID that has access
to team `4R7EAZY462`.

### Step 1 — Bump the version

Edit `ios/project.yml`:

```yaml
MARKETING_VERSION: "1.1.0"
CURRENT_PROJECT_VERSION: "2"   # must be higher than last accepted build
```

### Step 2 — Build the Rust xcframework

```bash
./scripts/build-ios.sh --device --release
```

This produces `ios/ZedraFFI.xcframework/ios-arm64/libzedra.a`.
The `.a` is gitignored and must be rebuilt whenever Rust code changes.

### Step 3 — Generate the Xcode project and install pods

```bash
cd ios
xcodegen generate
pod install
cd ..
```

`xcodegen generate` is required whenever `ios/project.yml` changes.
`pod install` produces `ios/Zedra.xcworkspace`, which is what Xcode and
xcodebuild use (not the bare `.xcodeproj`).

### Step 4 — Archive

**Via Xcode (recommended for first-time setup):**

1. `open ios/Zedra.xcworkspace`
2. Select **Any iOS Device (arm64)** as the destination (top-left device picker)
3. **Product → Archive**
4. Xcode opens the Organizer automatically when the archive completes

**Via command line:**

```bash
xcodebuild archive \
  -workspace ios/Zedra.xcworkspace \
  -scheme Zedra \
  -configuration Release \
  -destination "generic/platform=iOS" \
  -archivePath /tmp/Zedra.xcarchive \
  -allowProvisioningUpdates \
  IPHONEOS_DEPLOYMENT_TARGET="16.0"
```

`-allowProvisioningUpdates` lets Xcode download or refresh the provisioning
profile automatically. You must be signed into Xcode with a valid Apple ID.

### Step 5 — Distribute

**Via Xcode Organizer (easiest):**

1. **Window → Organizer** (or the Organizer opens automatically after archiving)
2. Select the archive → **Distribute App**
3. Choose **App Store Connect** → **Upload**
4. Follow the wizard — Xcode validates, re-signs if needed, and uploads
5. The build appears in TestFlight within a few minutes (processing takes ~5-15 min)

**Via command line (scriptable):**

```bash
# Export IPA from the archive
xcodebuild -exportArchive \
  -archivePath /tmp/Zedra.xcarchive \
  -exportOptionsPlist ios/ExportOptions.plist \
  -exportPath /tmp/ZedraExport \
  -allowProvisioningUpdates

# Upload to App Store Connect
# Requires the .p8 key at the well-known path, or pass --apiKey / --apiIssuer
xcrun altool --upload-app \
  --type ios \
  --file /tmp/ZedraExport/Zedra.ipa \
  --apiKey YOUR_KEY_ID \
  --apiIssuer YOUR_ISSUER_ID
```

Alternatively, drag `Zedra.ipa` into **Transporter.app** (free on the Mac App Store)
and click **Deliver** — no API key or command line needed.

### Step 6 — Verify in App Store Connect

1. App Store Connect → **Apps → Zedra → TestFlight**
2. The new build appears under the iOS builds list (status: Processing → Ready to Submit)
3. Add it to an internal group to start testing
