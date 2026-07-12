import { describe, it, expect, vi } from "vitest";
import { exponentialBackoff } from "./reconnect";

describe("exponentialBackoff", () => {
  it("should succeed on first attempt", async () => {
    const fn = vi.fn().mockResolvedValue(undefined);
    
    const result = await exponentialBackoff(fn, 3);
    
    expect(result.success).toBe(true);
    expect(result.attempts).toBe(1);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("should retry on failure and eventually succeed", async () => {
    const fn = vi
      .fn()
      .mockRejectedValueOnce(new Error("fail 1"))
      .mockRejectedValueOnce(new Error("fail 2"))
      .mockResolvedValue(undefined);
    
    const result = await exponentialBackoff(fn, 5);
    
    expect(result.success).toBe(true);
    expect(result.attempts).toBe(3);
    expect(fn).toHaveBeenCalledTimes(3);
  });

  it("should fail after max attempts", async () => {
    const error = new Error("persistent failure");
    const fn = vi.fn().mockRejectedValue(error);
    
    const result = await exponentialBackoff(fn, 3);
    
    expect(result.success).toBe(false);
    expect(result.attempts).toBe(3);
    expect(result.error).toEqual(error);
    expect(fn).toHaveBeenCalledTimes(3);
  });

  it("should use exponential delay between retries", async () => {
    const fn = vi
      .fn()
      .mockRejectedValueOnce(new Error("fail"))
      .mockResolvedValue(undefined);
    
    const start = Date.now();
    await exponentialBackoff(fn, 3);
    const duration = Date.now() - start;
    
    // Should have delayed at least 500ms (base delay)
    expect(duration).toBeGreaterThanOrEqual(450);
  });
});
