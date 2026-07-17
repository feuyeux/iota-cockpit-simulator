import { describe, it, expect } from "vitest";
import { redactValue, isSensitiveKey, REDACTED_SECRET } from "./redact";

describe("redact utilities", () => {
  it("flags sensitive keys regardless of case and separators", () => {
    expect(isSensitiveKey("apiKey")).toBe(true);
    expect(isSensitiveKey("api_key")).toBe(true);
    expect(isSensitiveKey("AUTH_TOKEN")).toBe(true);
    expect(isSensitiveKey("hidden-reasoning")).toBe(true);
    expect(isSensitiveKey("userPrompt")).toBe(true);
    expect(isSensitiveKey("credential")).toBe(true);
    expect(isSensitiveKey("credentials")).toBe(true);
    expect(isSensitiveKey("awsCredentials")).toBe(true);
    expect(isSensitiveKey("runId")).toBe(false);
    expect(isSensitiveKey("tick")).toBe(false);
  });

  it("recursively redacts nested secrets without mutating the input", () => {
    const input = {
      outer: {
        apiKey: "do-not-leak",
        nested: [{ auth_token: "also-do-not-leak" }],
        prompt: "private-prompt",
        safe: "keep-me",
      },
    };
    const redacted = redactValue(input);

    expect(redacted.outer.apiKey).toBe(REDACTED_SECRET);
    expect(redacted.outer.nested[0].auth_token).toBe(REDACTED_SECRET);
    expect(redacted.outer.prompt).toBe(REDACTED_SECRET);
    expect(redacted.outer.safe).toBe("keep-me");
    // Input is untouched.
    expect(input.outer.apiKey).toBe("do-not-leak");
    // Serialized artifact carries no secret substrings.
    const serialized = JSON.stringify(redacted);
    expect(serialized).not.toContain("do-not-leak");
    expect(serialized).not.toContain("also-do-not-leak");
    expect(serialized).not.toContain("private-prompt");
  });

  it("leaves scalars and arrays of scalars intact", () => {
    expect(redactValue(42)).toBe(42);
    expect(redactValue("plain")).toBe("plain");
    expect(redactValue([1, 2, 3])).toEqual([1, 2, 3]);
  });
});
