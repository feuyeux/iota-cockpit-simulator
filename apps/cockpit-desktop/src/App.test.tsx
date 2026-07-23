import { afterEach, describe, expect, it } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { App } from "./App";
import { I18nProvider } from "./i18n";
import { ThemeProvider } from "./theme";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root!.render(
      <ThemeProvider>
        <I18nProvider>
          <App />
        </I18nProvider>
      </ThemeProvider>
    );
  });
  return container;
}

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  window.localStorage.clear();
});

describe("App workspace", () => {
  it("keeps Activity visible and opens insights only on demand", () => {
    const element = render();

    expect(element.textContent).toContain("仿真经过");
    expect(element.textContent).not.toContain("仿真过程与评测");

    const openInsights = element.querySelector('button[aria-label="评测"]') as HTMLButtonElement;
    act(() => openInsights.click());

    expect(element.textContent).toContain("仿真过程与评测");
    expect(element.textContent).toContain("场景交互");
    expect(element.textContent).toContain("仿真经过");
  });
});
