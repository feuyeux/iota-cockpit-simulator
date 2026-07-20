/// Tauri's `invoke()` rejects with whatever the Rust command returned as its
/// error type. Every command in this app returns `Result<_, String>`, so the
/// rejection value is a **plain string**, not a JS `Error` instance. Code that
/// only checked `error instanceof Error` therefore always fell through to a
/// generic fallback message and silently discarded the actual backend
/// diagnostic (e.g. "LIVE_BACKEND_TURN_FAILED: hermes unreachable"), leaving
/// users with an unhelpful generic error and no way to tell what went wrong.
///
/// This normalizes all the shapes we can realistically receive:
/// - a plain string (the common Tauri case)
/// - an `Error` instance (thrown by our own client code, e.g. simulatorClient)
/// - an object with a `message` string field (defensive fallback)
/// - anything else falls back to the provided default.
export function describeError(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim().length > 0) {
    return error;
  }
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as { message: unknown }).message === "string" &&
    (error as { message: string }).message.trim().length > 0
  ) {
    return (error as { message: string }).message;
  }
  return fallback;
}
