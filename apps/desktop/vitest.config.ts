import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['src/main/**/*.test.ts', 'tests/main/**/*.test.ts'],
    environment: 'node',
    globals: true,
    // Reclaims every mkdtemp sandbox a test file creates. /tmp is a per-user
    // quota tmpfs here, and a full one stops the shell being able to exec at
    // all — see tests/support/tmpdirCleanup.ts.
    setupFiles: ['./tests/support/tmpdirCleanup.ts'],
  },
});
