import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['src/main/**/*.test.ts', 'tests/main/**/*.test.ts'],
    environment: 'node',
    globals: true,
  },
});
