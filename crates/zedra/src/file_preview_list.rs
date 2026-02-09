// FilePreviewList — grid of preview cards for sample code files.
// Tapping a card emits PreviewSelected so the parent can push an EditorView.

use gpui::prelude::FluentBuilder;
use gpui::*;

/// Metadata for a sample file shown in the preview grid.
pub struct SampleFile {
    pub filename: &'static str,
    pub language: &'static str,
    pub content: &'static str,
    pub line_count: usize,
}

// ============================================================================
// RUST SAMPLES
// ============================================================================

const SAMPLE_CACHE_RS: &str = r#"use std::collections::HashMap;

/// A simple key-value store with expiration.
pub struct Cache<V> {
    entries: HashMap<String, (V, Option<std::time::Instant>)>,
    default_ttl: std::time::Duration,
}

impl<V: Clone> Cache<V> {
    pub fn new(default_ttl: std::time::Duration) -> Self {
        Self {
            entries: HashMap::new(),
            default_ttl,
        }
    }

    pub fn insert(&mut self, key: String, value: V) {
        let expires_at = std::time::Instant::now() + self.default_ttl;
        self.entries.insert(key, (value, Some(expires_at)));
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        match self.entries.get(key) {
            Some((value, Some(exp))) if *exp > std::time::Instant::now() => Some(value),
            Some((value, None)) => Some(value),
            _ => None,
        }
    }

    pub fn evict_expired(&mut self) {
        let now = std::time::Instant::now();
        self.entries.retain(|_, (_, exp)| exp.map_or(true, |e| e > now));
    }
}

fn main() {
    let mut cache = Cache::new(std::time::Duration::from_secs(60));
    cache.insert("greeting".to_string(), "Hello, world!");
    if let Some(v) = cache.get("greeting") {
        println!("Found: {}", v);
    }
}"#;

// ============================================================================
// PYTHON SAMPLES
// ============================================================================

const SAMPLE_PYTHON: &str = r#"from dataclasses import dataclass
from typing import Optional, List
import asyncio

@dataclass
class Task:
    """Represents an async task with priority."""
    id: int
    name: str
    priority: int = 0
    completed: bool = False

class TaskQueue:
    def __init__(self, max_concurrent: int = 5):
        self.tasks: List[Task] = []
        self.semaphore = asyncio.Semaphore(max_concurrent)

    async def add_task(self, task: Task) -> None:
        self.tasks.append(task)
        self.tasks.sort(key=lambda t: -t.priority)

    async def process_next(self) -> Optional[Task]:
        async with self.semaphore:
            if not self.tasks:
                return None
            task = self.tasks.pop(0)
            await self._execute(task)
            return task

    async def _execute(self, task: Task) -> None:
        print(f"Executing: {task.name}")
        await asyncio.sleep(0.1)
        task.completed = True

async def main():
    queue = TaskQueue(max_concurrent=3)
    for i in range(10):
        await queue.add_task(Task(id=i, name=f"Task-{i}", priority=i % 3))

    while queue.tasks:
        await queue.process_next()

if __name__ == "__main__":
    asyncio.run(main())
"#;

// ============================================================================
// GO SAMPLES
// ============================================================================

const SAMPLE_GO: &str = r#"package main

import (
	"context"
	"fmt"
	"sync"
	"time"
)

// Worker processes jobs from a channel
type Worker struct {
	ID       int
	JobQueue <-chan Job
	Results  chan<- Result
	wg       *sync.WaitGroup
}

type Job struct {
	ID      int
	Payload string
}

type Result struct {
	JobID   int
	Output  string
	Elapsed time.Duration
}

func (w *Worker) Start(ctx context.Context) {
	defer w.wg.Done()
	for {
		select {
		case <-ctx.Done():
			return
		case job, ok := <-w.JobQueue:
			if !ok {
				return
			}
			start := time.Now()
			output := fmt.Sprintf("Worker %d processed: %s", w.ID, job.Payload)
			w.Results <- Result{
				JobID:   job.ID,
				Output:  output,
				Elapsed: time.Since(start),
			}
		}
	}
}

func main() {
	jobs := make(chan Job, 100)
	results := make(chan Result, 100)
	var wg sync.WaitGroup

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	for i := 0; i < 3; i++ {
		wg.Add(1)
		worker := &Worker{ID: i, JobQueue: jobs, Results: results, wg: &wg}
		go worker.Start(ctx)
	}

	for i := 0; i < 10; i++ {
		jobs <- Job{ID: i, Payload: fmt.Sprintf("data-%d", i)}
	}
	close(jobs)

	go func() {
		wg.Wait()
		close(results)
	}()

	for r := range results {
		fmt.Printf("Job %d: %s (%v)\n", r.JobID, r.Output, r.Elapsed)
	}
}
"#;

// ============================================================================
// JAVASCRIPT SAMPLES
// ============================================================================

const SAMPLE_JAVASCRIPT: &str = r#"class EventEmitter {
  constructor() {
    this.events = new Map();
  }

  on(event, callback) {
    if (!this.events.has(event)) {
      this.events.set(event, []);
    }
    this.events.get(event).push(callback);
    return () => this.off(event, callback);
  }

  off(event, callback) {
    const handlers = this.events.get(event);
    if (handlers) {
      const index = handlers.indexOf(callback);
      if (index > -1) handlers.splice(index, 1);
    }
  }

  emit(event, ...args) {
    const handlers = this.events.get(event) || [];
    handlers.forEach(handler => handler(...args));
  }

  once(event, callback) {
    const wrapper = (...args) => {
      callback(...args);
      this.off(event, wrapper);
    };
    return this.on(event, wrapper);
  }
}

// Usage example
const emitter = new EventEmitter();

const unsubscribe = emitter.on('message', (msg) => {
  console.log(`Received: ${msg}`);
});

emitter.once('connect', () => {
  console.log('Connected!');
});

emitter.emit('connect');
emitter.emit('message', 'Hello, World!');
emitter.emit('message', 'Goodbye!');
unsubscribe();
emitter.emit('message', 'This will not be logged');
"#;

// ============================================================================
// TYPESCRIPT SAMPLES
// ============================================================================

const SAMPLE_TYPESCRIPT: &str = r#"interface Repository<T> {
  findById(id: string): Promise<T | null>;
  findAll(): Promise<T[]>;
  save(entity: T): Promise<T>;
  delete(id: string): Promise<boolean>;
}

interface User {
  id: string;
  name: string;
  email: string;
  createdAt: Date;
}

class InMemoryUserRepository implements Repository<User> {
  private users: Map<string, User> = new Map();

  async findById(id: string): Promise<User | null> {
    return this.users.get(id) ?? null;
  }

  async findAll(): Promise<User[]> {
    return Array.from(this.users.values());
  }

  async save(user: User): Promise<User> {
    this.users.set(user.id, user);
    return user;
  }

  async delete(id: string): Promise<boolean> {
    return this.users.delete(id);
  }
}

class UserService {
  constructor(private readonly repository: Repository<User>) {}

  async createUser(name: string, email: string): Promise<User> {
    const user: User = {
      id: crypto.randomUUID(),
      name,
      email,
      createdAt: new Date(),
    };
    return this.repository.save(user);
  }

  async getUserById(id: string): Promise<User | null> {
    return this.repository.findById(id);
  }
}

async function main() {
  const repo = new InMemoryUserRepository();
  const service = new UserService(repo);

  const user = await service.createUser("Alice", "alice@example.com");
  console.log("Created:", user);

  const found = await service.getUserById(user.id);
  console.log("Found:", found);
}

main();
"#;

// ============================================================================
// C SAMPLES
// ============================================================================

const SAMPLE_C: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_SIZE 1024

typedef struct Node {
    int data;
    struct Node* next;
} Node;

typedef struct {
    Node* head;
    Node* tail;
    size_t size;
} LinkedList;

LinkedList* list_create(void) {
    LinkedList* list = malloc(sizeof(LinkedList));
    if (list) {
        list->head = NULL;
        list->tail = NULL;
        list->size = 0;
    }
    return list;
}

int list_push(LinkedList* list, int value) {
    Node* node = malloc(sizeof(Node));
    if (!node) return -1;

    node->data = value;
    node->next = NULL;

    if (list->tail) {
        list->tail->next = node;
    } else {
        list->head = node;
    }
    list->tail = node;
    list->size++;
    return 0;
}

int list_pop(LinkedList* list) {
    if (!list->head) return -1;

    Node* node = list->head;
    int value = node->data;
    list->head = node->next;

    if (!list->head) {
        list->tail = NULL;
    }

    free(node);
    list->size--;
    return value;
}

void list_free(LinkedList* list) {
    while (list->head) {
        list_pop(list);
    }
    free(list);
}

int main(void) {
    LinkedList* list = list_create();

    for (int i = 0; i < 10; i++) {
        list_push(list, i * 2);
    }

    printf("List size: %zu\n", list->size);

    while (list->size > 0) {
        printf("Popped: %d\n", list_pop(list));
    }

    list_free(list);
    return 0;
}
"#;

// ============================================================================
// C++ SAMPLES
// ============================================================================

const SAMPLE_CPP: &str = r#"#include <iostream>
#include <memory>
#include <vector>
#include <algorithm>
#include <functional>

template<typename T>
class Observable {
public:
    using Observer = std::function<void(const T&)>;

    void subscribe(Observer observer) {
        observers_.push_back(std::move(observer));
    }

    void notify(const T& value) {
        for (const auto& observer : observers_) {
            observer(value);
        }
    }

private:
    std::vector<Observer> observers_;
};

class Sensor {
public:
    explicit Sensor(std::string name) : name_(std::move(name)) {}

    void updateValue(double value) {
        value_ = value;
        observable_.notify(value);
    }

    void addObserver(Observable<double>::Observer observer) {
        observable_.subscribe(std::move(observer));
    }

    const std::string& name() const { return name_; }
    double value() const { return value_; }

private:
    std::string name_;
    double value_ = 0.0;
    Observable<double> observable_;
};

class Dashboard {
public:
    void addSensor(std::shared_ptr<Sensor> sensor) {
        sensor->addObserver([name = sensor->name()](double value) {
            std::cout << name << ": " << value << std::endl;
        });
        sensors_.push_back(std::move(sensor));
    }

private:
    std::vector<std::shared_ptr<Sensor>> sensors_;
};

int main() {
    auto dashboard = std::make_unique<Dashboard>();

    auto temp = std::make_shared<Sensor>("Temperature");
    auto humidity = std::make_shared<Sensor>("Humidity");

    dashboard->addSensor(temp);
    dashboard->addSensor(humidity);

    temp->updateValue(22.5);
    humidity->updateValue(65.0);
    temp->updateValue(23.1);

    return 0;
}
"#;

// ============================================================================
// CSS SAMPLES
// ============================================================================

const SAMPLE_CSS: &str = r#":root {
  --primary-color: #3498db;
  --secondary-color: #2ecc71;
  --background: #1a1a2e;
  --surface: #16213e;
  --text-primary: #eee;
  --text-secondary: #aaa;
  --spacing-unit: 8px;
  --border-radius: 4px;
}

* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
  background: var(--background);
  color: var(--text-primary);
  line-height: 1.6;
}

.container {
  max-width: 1200px;
  margin: 0 auto;
  padding: calc(var(--spacing-unit) * 2);
}

.card {
  background: var(--surface);
  border-radius: var(--border-radius);
  padding: calc(var(--spacing-unit) * 3);
  box-shadow: 0 4px 6px rgba(0, 0, 0, 0.3);
  transition: transform 0.2s ease, box-shadow 0.2s ease;
}

.card:hover {
  transform: translateY(-2px);
  box-shadow: 0 8px 25px rgba(0, 0, 0, 0.4);
}

.button {
  display: inline-flex;
  align-items: center;
  gap: var(--spacing-unit);
  padding: calc(var(--spacing-unit) * 1.5) calc(var(--spacing-unit) * 3);
  background: var(--primary-color);
  color: white;
  border: none;
  border-radius: var(--border-radius);
  cursor: pointer;
  font-weight: 500;
}

.button:hover {
  background: color-mix(in srgb, var(--primary-color) 85%, black);
}

@media (max-width: 768px) {
  .container {
    padding: var(--spacing-unit);
  }

  .card {
    padding: calc(var(--spacing-unit) * 2);
  }
}
"#;

// ============================================================================
// JSON SAMPLES
// ============================================================================

const SAMPLE_JSON: &str = r#"{
  "name": "zedra-mobile",
  "version": "0.1.0",
  "description": "GPUI on Android with Vulkan rendering",
  "repository": {
    "type": "git",
    "url": "https://github.com/example/zedra"
  },
  "engines": {
    "node": ">=18.0.0"
  },
  "scripts": {
    "build": "cargo build --release",
    "test": "cargo test",
    "lint": "cargo clippy -- -D warnings",
    "deploy": "./scripts/build-android.sh && ./scripts/install.sh"
  },
  "android": {
    "minSdk": 26,
    "targetSdk": 34,
    "vulkan": {
      "minVersion": "1.1",
      "features": ["traditional_renderpass"]
    }
  },
  "dependencies": {
    "gpui": "workspace",
    "blade": "0.3.0",
    "tree-sitter": "0.26"
  },
  "devDependencies": {
    "android-ndk": "r25c"
  },
  "keywords": ["android", "gpu", "vulkan", "ui", "mobile"],
  "author": "Zedra Team",
  "license": "MIT"
}
"#;

// ============================================================================
// YAML SAMPLES
// ============================================================================

const SAMPLE_YAML: &str = r#"name: CI/CD Pipeline

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        rust: [stable, beta, nightly]
        target:
          - x86_64-unknown-linux-gnu
          - aarch64-linux-android

    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive

      - name: Install Rust
        uses: dtolnay/rust-action@stable
        with:
          toolchain: ${{ matrix.rust }}
          target: ${{ matrix.target }}

      - name: Cache cargo
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: Build
        run: cargo build --release --target ${{ matrix.target }}

      - name: Run tests
        if: matrix.target == 'x86_64-unknown-linux-gnu'
        run: cargo test --release

  deploy:
    needs: build
    runs-on: ubuntu-latest
    if: github.ref == 'refs/heads/main'

    steps:
      - name: Deploy to production
        run: echo "Deploying..."
"#;

// ============================================================================
// BASH SAMPLES
// ============================================================================

const SAMPLE_BASH: &str = r#"#!/usr/bin/env bash
set -euo pipefail

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
readonly BUILD_DIR="${PROJECT_ROOT}/target"
readonly LOG_FILE="${BUILD_DIR}/build.log"

# Colors for output
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly NC='\033[0m'

log_info() {
    echo -e "${GREEN}[INFO]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*" >&2
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

check_dependencies() {
    local deps=("cargo" "rustc" "adb")
    local missing=()

    for dep in "${deps[@]}"; do
        if ! command -v "$dep" &> /dev/null; then
            missing+=("$dep")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        exit 1
    fi
}

build_project() {
    local target="${1:-aarch64-linux-android}"

    log_info "Building for target: $target"

    mkdir -p "$BUILD_DIR"

    if cargo build --release --target "$target" 2>&1 | tee "$LOG_FILE"; then
        log_info "Build completed successfully"
        return 0
    else
        log_error "Build failed. Check $LOG_FILE for details."
        return 1
    fi
}

main() {
    check_dependencies

    case "${1:-build}" in
        build)
            build_project "${2:-aarch64-linux-android}"
            ;;
        clean)
            log_info "Cleaning build directory..."
            rm -rf "$BUILD_DIR"
            ;;
        *)
            log_error "Unknown command: $1"
            echo "Usage: $0 {build|clean} [target]"
            exit 1
            ;;
    esac
}

main "$@"
"#;

// ============================================================================
// MARKDOWN SAMPLES
// ============================================================================

const SAMPLE_MARKDOWN: &str = r#"# Zedra Architecture Guide

> Bringing GPUI to Android with Vulkan 1.1

## Overview

Zedra is a port of Zed's **GPUI** framework to Android, achieving 60 FPS rendering with full touch input support.

### Key Features

- Vulkan 1.1 traditional renderpass (90% device compatibility)
- Thread-safe command queue architecture
- CosmicText for text rendering
- Tree-sitter syntax highlighting

## Quick Start

```bash
# Build and deploy
./scripts/dev-cycle.sh

# View logs
adb logcat | grep zedra
```

## Architecture

| Component | Description |
|-----------|-------------|
| `android_jni.rs` | JNI bridge for Java ↔ Rust |
| `android_app.rs` | Main thread GPUI application |
| `command_queue.rs` | Thread-safe event queue |

### Threading Model

1. **JNI Thread** → Receives Android events
2. **Command Queue** → Thread-safe buffer
3. **Main Thread** → GPUI rendering at 60 FPS

## Performance

- Platform init: ~51ms
- CPU per frame: <5ms
- Memory: ~40-50 MB

---

*For more details, see the [full documentation](docs/ARCHITECTURE.md).*
"#;

// ============================================================================
// TSX SAMPLE
// ============================================================================

const SAMPLE_TSX: &str = r#"import React, { useState, useCallback, useMemo } from 'react';

interface TodoItem {
  id: number;
  text: string;
  completed: boolean;
}

interface TodoListProps {
  initialItems?: TodoItem[];
}

const TodoList: React.FC<TodoListProps> = ({ initialItems = [] }) => {
  const [items, setItems] = useState<TodoItem[]>(initialItems);
  const [input, setInput] = useState('');

  const addItem = useCallback(() => {
    if (!input.trim()) return;

    setItems(prev => [
      ...prev,
      { id: Date.now(), text: input.trim(), completed: false }
    ]);
    setInput('');
  }, [input]);

  const toggleItem = useCallback((id: number) => {
    setItems(prev =>
      prev.map(item =>
        item.id === id ? { ...item, completed: !item.completed } : item
      )
    );
  }, []);

  const stats = useMemo(() => ({
    total: items.length,
    completed: items.filter(i => i.completed).length,
    pending: items.filter(i => !i.completed).length,
  }), [items]);

  return (
    <div className="todo-list">
      <h1>Todo List</h1>
      <div className="stats">
        <span>Total: {stats.total}</span>
        <span>Done: {stats.completed}</span>
        <span>Pending: {stats.pending}</span>
      </div>
      <div className="input-row">
        <input
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyPress={e => e.key === 'Enter' && addItem()}
          placeholder="Add new todo..."
        />
        <button onClick={addItem}>Add</button>
      </div>
      <ul>
        {items.map(item => (
          <li
            key={item.id}
            onClick={() => toggleItem(item.id)}
            className={item.completed ? 'completed' : ''}
          >
            {item.text}
          </li>
        ))}
      </ul>
    </div>
  );
};

export default TodoList;
"#;

// ============================================================================
// SAMPLE FILES ARRAY
// ============================================================================

pub const SAMPLE_FILES: &[SampleFile] = &[
    // Rust
    SampleFile {
        filename: "cache.rs",
        language: "Rust",
        content: SAMPLE_CACHE_RS,
        line_count: 56,
    },
    // Python
    SampleFile {
        filename: "task_queue.py",
        language: "Python",
        content: SAMPLE_PYTHON,
        line_count: 52,
    },
    // Go
    SampleFile {
        filename: "worker.go",
        language: "Go",
        content: SAMPLE_GO,
        line_count: 75,
    },
    // JavaScript
    SampleFile {
        filename: "emitter.js",
        language: "JavaScript",
        content: SAMPLE_JAVASCRIPT,
        line_count: 52,
    },
    // TypeScript
    SampleFile {
        filename: "repository.ts",
        language: "TypeScript",
        content: SAMPLE_TYPESCRIPT,
        line_count: 62,
    },
    // TSX
    SampleFile {
        filename: "TodoList.tsx",
        language: "TSX",
        content: SAMPLE_TSX,
        line_count: 72,
    },
    // C
    SampleFile {
        filename: "linked_list.c",
        language: "C",
        content: SAMPLE_C,
        line_count: 78,
    },
    // C++
    SampleFile {
        filename: "observer.cpp",
        language: "C++",
        content: SAMPLE_CPP,
        line_count: 72,
    },
    // CSS
    SampleFile {
        filename: "theme.css",
        language: "CSS",
        content: SAMPLE_CSS,
        line_count: 68,
    },
    // JSON
    SampleFile {
        filename: "package.json",
        language: "JSON",
        content: SAMPLE_JSON,
        line_count: 38,
    },
    // YAML
    SampleFile {
        filename: "ci.yml",
        language: "YAML",
        content: SAMPLE_YAML,
        line_count: 62,
    },
    // Bash
    SampleFile {
        filename: "build.sh",
        language: "Bash",
        content: SAMPLE_BASH,
        line_count: 74,
    },
    // Markdown
    SampleFile {
        filename: "ARCHITECTURE.md",
        language: "Markdown",
        content: SAMPLE_MARKDOWN,
        line_count: 52,
    },
];

/// Event emitted when a preview card is tapped.
#[derive(Clone, Debug)]
pub struct PreviewSelected {
    pub index: usize,
}

pub struct FilePreviewList {
    focus_handle: FocusHandle,
}

impl FilePreviewList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PreviewSelected> for FilePreviewList {}

impl Focusable for FilePreviewList {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FilePreviewList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div().flex().flex_row().flex_wrap().gap_3().p_4();

        for (idx, sample) in SAMPLE_FILES.iter().enumerate() {
            // First 6 lines of code, truncated to ~24 chars each
            let preview_lines: Vec<String> = sample
                .content
                .lines()
                .take(6)
                .map(|l| {
                    if l.len() > 24 {
                        format!("{}...", &l[..24])
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            let preview_text = preview_lines.join("\n");
            let line_count_label = format!("{} lines", sample.line_count);
            let filename: SharedString = sample.filename.into();
            let language: SharedString = sample.language.into();

            // Color-code language badges
            let badge_color = match sample.language {
                "Rust" => rgb(0xdea584),
                "Python" => rgb(0x3572a5),
                "Go" => rgb(0x00add8),
                "JavaScript" => rgb(0xf1e05a),
                "TypeScript" => rgb(0x3178c6),
                "TSX" => rgb(0x3178c6),
                "C" => rgb(0x555555),
                "C++" => rgb(0xf34b7d),
                "CSS" => rgb(0x563d7c),
                "JSON" => rgb(0x292929),
                "YAML" => rgb(0xcb171e),
                "Bash" => rgb(0x89e051),
                "Markdown" => rgb(0x083fa1),
                _ => rgb(0x3e4451),
            };

            grid = grid.child(
                div()
                    .w(px(155.0))
                    .h(px(180.0))
                    .bg(rgb(0x282c34))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(0x3e4451))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgb(0x61afef)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            cx.emit(PreviewSelected { index: idx });
                        }),
                    )
                    // Filename
                    .child(div().text_sm().text_color(rgb(0xabb2bf)).child(filename))
                    // Language badge
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(badge_color)
                            .text_xs()
                            .text_color(rgb(0xffffff))
                            .child(language),
                    )
                    // Code preview
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(rgb(0x5c6370))
                            .child(preview_text),
                    )
                    // Line count
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x4b5263))
                            .child(line_count_label),
                    ),
            );
        }

        div()
            .id("file-preview-list")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .overflow_y_scroll()
            .child(
                div()
                    .p_4()
                    .child(
                        div()
                            .text_color(rgb(0x61afef))
                            .text_lg()
                            .child("Code Samples"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x5c6370))
                            .text_sm()
                            .mt_1()
                            .child("Tap a file to open in editor"),
                    ),
            )
            .child(grid)
    }
}
