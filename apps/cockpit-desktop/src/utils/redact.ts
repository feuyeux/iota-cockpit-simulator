// Client-side secret redaction for every export/artifact path.
//
// The Rust trace boundary already redacts before emitting, but exports,
// screenshots, and issue/runbook output are separate egress paths and must
// redact defensively so a secret injected downstream never reaches disk.
// This mirrors `redact_json` / `sensitive_key` in `cockpit-agent-runtime`.

export const REDACTED_SECRET = "[REDACTED]";

const SENSITIVE_KEYS = new Set([
  "apikey",
  "token",
  "authorization",
  "password",
  "secret",
  "prompt",
  "reasoning",
  "hiddenreasoning",
  "chainofthought",
]);

const SENSITIVE_SUFFIXES = ["apikey", "token", "secret", "password", "prompt"];

function normalizeKey(key: string): string {
  return key.replace(/[^a-z0-9]/gi, "").toLowerCase();
}

export function isSensitiveKey(key: string): boolean {
  const normalized = normalizeKey(key);
  if (SENSITIVE_KEYS.has(normalized)) {
    return true;
  }
  return SENSITIVE_SUFFIXES.some((suffix) => normalized.endsWith(suffix));
}

/**
 * Recursively redact sensitive fields in an arbitrary JSON-like value.
 * Returns a redacted deep copy; the input is not mutated.
 */
export function redactValue<T>(value: T): T {
  if (Array.isArray(value)) {
    return value.map((item) => redactValue(item)) as unknown as T;
  }
  if (value !== null && typeof value === "object") {
    const source = value as Record<string, unknown>;
    const output: Record<string, unknown> = {};
    for (const [key, inner] of Object.entries(source)) {
      output[key] = isSensitiveKey(key) ? REDACTED_SECRET : redactValue(inner);
    }
    return output as unknown as T;
  }
  return value;
}
