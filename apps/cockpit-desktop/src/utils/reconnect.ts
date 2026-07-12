import { APP_CONFIG } from "../config/constants";

export async function exponentialBackoff(
  fn: () => Promise<void>,
  maxAttempts: number = APP_CONFIG.RECONNECT_MAX_ATTEMPTS
): Promise<{ success: boolean; attempts: number; error?: Error }> {
  let attempts = 0;
  let delay: number = APP_CONFIG.RECONNECT_BASE_DELAY;

  while (attempts < maxAttempts) {
    attempts++;
    try {
      await fn();
      return { success: true, attempts };
    } catch (error) {
      if (attempts >= maxAttempts) {
        return {
          success: false,
          attempts,
          error: error instanceof Error ? error : new Error("Unknown error"),
        };
      }
      await new Promise((resolve) => setTimeout(resolve, delay));
      delay = Math.min(delay * 2, APP_CONFIG.RECONNECT_MAX_DELAY);
    }
  }

  return { success: false, attempts };
}
