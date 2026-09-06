import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['tests/integration/dispatchChain.integration.ts'],
    environment: 'node',
    hookTimeout: 180_000,
    testTimeout: 30_000,
  },
});
