import { SELF } from "cloudflare:test";
import { describe, it, expect } from "vitest";

describe("HTTP routes", () => {
  it("GET / returns health check", async () => {
    const resp = await SELF.fetch("http://localhost/");
    expect(resp.status).toBe(200);
    const body = (await resp.json()) as { ok: boolean };
    expect(body).toEqual({ ok: true });
  });

  it("GET /ping returns pong", async () => {
    const resp = await SELF.fetch("http://localhost/ping");
    expect(resp.status).toBe(200);
    const text = await resp.text();
    expect(text).toBe("pong");
  });

  it("GET /generate_204 returns 204", async () => {
    const resp = await SELF.fetch("http://localhost/generate_204");
    expect(resp.status).toBe(204);
  });

  it("GET /relay without Upgrade header returns 426", async () => {
    const resp = await SELF.fetch("http://localhost/relay");
    expect(resp.status).toBe(426);
  });

  it("GET /unknown returns 404", async () => {
    const resp = await SELF.fetch("http://localhost/unknown");
    expect(resp.status).toBe(404);
  });

  it("OPTIONS / returns 204 with CORS headers", async () => {
    const resp = await SELF.fetch("http://localhost/", { method: "OPTIONS" });
    expect(resp.status).toBe(204);
    expect(resp.headers.get("Access-Control-Allow-Origin")).toBe("*");
    expect(resp.headers.get("Access-Control-Allow-Methods")).toContain("GET");
  });
});
