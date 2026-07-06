import { useCallback } from 'react';
import { useConfig } from './useConfig';
import { MODE_MANIFEST, resolveUiMode, type ModeManifest, type UiMode } from '../lib/uiMode';

/**
 * The single seam between the UI-mode config (config.ui.mode) and the
 * components that render differently per mode. Consumers read the manifest
 * flags — never compare mode strings — so what each mode shows stays declared
 * in lib/uiMode.ts.
 */
export function useUiMode(): {
  mode: UiMode;
  manifest: ModeManifest;
  setMode: (next: UiMode) => void;
  toggle: () => void;
} {
  const { config, save } = useConfig();
  const mode = resolveUiMode(config.ui?.mode);

  const setMode = useCallback(
    (next: UiMode) => {
      void save({ ui: { ...config.ui, mode: next } });
    },
    [config.ui, save],
  );

  const toggle = useCallback(() => {
    setMode(mode === 'focus' ? 'fleet' : 'focus');
  }, [mode, setMode]);

  return { mode, manifest: MODE_MANIFEST[mode], setMode, toggle };
}
