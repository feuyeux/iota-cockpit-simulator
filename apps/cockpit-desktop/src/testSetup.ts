const values = new Map<string, string>();

const testStorage: Storage = {
  get length() {
    return values.size;
  },
  clear() {
    values.clear();
  },
  getItem(key) {
    return values.get(key) ?? null;
  },
  key(index) {
    return [...values.keys()][index] ?? null;
  },
  removeItem(key) {
    values.delete(key);
  },
  setItem(key, value) {
    values.set(key, String(value));
  },
};

Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: testStorage,
  writable: true,
});
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: testStorage,
  writable: true,
});

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
