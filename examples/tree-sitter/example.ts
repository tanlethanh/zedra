// Generic result type and utilities

type Ok<T> = { ok: true; value: T };
type Err<E> = { ok: false; error: E };
type Result<T, E = Error> = Ok<T> | Err<E>;

function ok<T>(value: T): Ok<T> {
  return { ok: true, value };
}

function err<E>(error: E): Err<E> {
  return { ok: false, error };
}

function unwrap<T, E>(result: Result<T, E>): T {
  if (result.ok) return result.value;
  throw result.error;
}

// Typed HTTP client

interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE" | "PATCH";
  headers?: Record<string, string>;
  body?: unknown;
  timeout?: number;
}

async function request<T>(
  url: string,
  options: RequestOptions = {}
): Promise<Result<T>> {
  const { method = "GET", headers = {}, body, timeout = 5000 } = options;

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);

  try {
    const res = await fetch(url, {
      method,
      headers: { "Content-Type": "application/json", ...headers },
      body: body != null ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });

    clearTimeout(timer);

    if (!res.ok) {
      return err(new Error(`HTTP ${res.status}: ${res.statusText}`));
    }

    const data: T = await res.json();
    return ok(data);
  } catch (e) {
    clearTimeout(timer);
    return err(e instanceof Error ? e : new Error(String(e)));
  }
}

// Debounce utility

function debounce<T extends (...args: unknown[]) => void>(
  fn: T,
  ms: number
): (...args: Parameters<T>) => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return (...args) => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

// Usage

interface User {
  id: number;
  name: string;
  email: string;
}

const search = debounce(async (query: string) => {
  const result = await request<User[]>(`/api/users?q=${query}`);
  if (result.ok) {
    console.log(result.value.map((u) => u.name));
  } else {
    console.error(result.error.message);
  }
}, 300);

search("alice");
