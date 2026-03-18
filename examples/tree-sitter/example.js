// Async task queue with concurrency limit

class TaskQueue {
  #queue = [];
  #running = 0;

  constructor(concurrency = 4) {
    this.concurrency = concurrency;
  }

  enqueue(fn) {
    return new Promise((resolve, reject) => {
      this.#queue.push({ fn, resolve, reject });
      this.#drain();
    });
  }

  #drain() {
    while (this.#running < this.concurrency && this.#queue.length > 0) {
      const { fn, resolve, reject } = this.#queue.shift();
      this.#running++;
      Promise.resolve()
        .then(() => fn())
        .then(resolve, reject)
        .finally(() => {
          this.#running--;
          this.#drain();
        });
    }
  }

  get size() {
    return this.#queue.length + this.#running;
  }
}

// Simple event emitter
class EventEmitter {
  #listeners = new Map();

  on(event, listener) {
    if (!this.#listeners.has(event)) {
      this.#listeners.set(event, []);
    }
    this.#listeners.get(event).push(listener);
    return () => this.off(event, listener);
  }

  off(event, listener) {
    const list = this.#listeners.get(event) ?? [];
    this.#listeners.set(event, list.filter((l) => l !== listener));
  }

  emit(event, ...args) {
    const list = this.#listeners.get(event) ?? [];
    for (const listener of list) {
      listener(...args);
    }
  }
}

async function fetchWithRetry(url, retries = 3, delay = 500) {
  for (let attempt = 0; attempt < retries; attempt++) {
    try {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return await res.json();
    } catch (err) {
      if (attempt === retries - 1) throw err;
      await new Promise((r) => setTimeout(r, delay * 2 ** attempt));
    }
  }
}

const queue = new TaskQueue(2);
const emitter = new EventEmitter();

emitter.on("done", (result) => console.log("done:", result));

const tasks = [1, 2, 3, 4, 5].map((n) =>
  queue.enqueue(() => new Promise((r) => setTimeout(() => r(n * n), 100 * n)))
);

Promise.all(tasks).then((results) => emitter.emit("done", results));
