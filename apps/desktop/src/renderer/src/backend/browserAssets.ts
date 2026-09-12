import type { ElectronAPI } from '../types/electron';
import { openBrowserFilePicker, readFileBase64 } from '../lib/attachmentUpload';

type Font = { file: string; family: string };
type Asset = { dataBase64: string; mime: string; family?: string };
export function browserAssets(call: <T>(method: string, params: unknown, timeout?: number) => Promise<T>, native = false) {
  const fonts = new Map<string, Promise<FontFace>>();
  const fontFaces = new Map<string, FontFace>();
  const list = () => call<Font[]>('ui.fonts', {});
  const asset = (kind: 'font' | 'icon', file: string) => call<Asset>('ui.asset', { kind, file }, 90_000);
  const load = (font: Font, replace = false): Promise<FontFace> => {
    if (replace) { fonts.delete(font.file); const old = fontFaces.get(font.file); if (old) document.fonts.delete(old); }
    let loading = fonts.get(font.file);
    if (!loading) {
      loading = asset('font', font.file).then(async (data) => {
        const bytes = Uint8Array.from(atob(data.dataBase64), (c) => c.charCodeAt(0));
        const face = new FontFace(JSON.stringify(font.family), bytes.buffer, { weight: '100 900', display: 'swap' });
        await face.load(); document.fonts.add(face); fontFaces.set(font.file, face); return face;
      });
      fonts.set(font.file, loading);
      void loading.catch(() => { fonts.delete(font.file); });
    }
    return loading;
  };
  const api: Pick<ElectronAPI, 'getUiAsset' | 'listUiFonts' | 'installUiFont' | 'downloadProjectIcon'> = {
    getUiAsset: asset,
    listUiFonts: list,
    installUiFont: async () => {
      const [file] = await openBrowserFilePicker('.ttf,.otf,.woff,.woff2', false);
      if (!file) return null;
      if (file.size > 12 * 1024 * 1024) throw new Error('Choose a font file up to 12 MiB');
      const installed = await call<Font>('desktop.installUiFont', { name: file.name, dataBase64: await readFileBase64(file) }, 90_000);
      await load(installed, true);
      return installed;
    },
    downloadProjectIcon: async (url) => {
      try { return await call('desktop.downloadProjectIcon', { url }); }
      catch (error) { return { ok: false, error: error instanceof Error ? error.message : 'Icon download failed' }; }
    },
  };
  return {
    api,
    ensureFont: async (family?: string) => {
      if (native || !family?.startsWith('custom:') || typeof FontFace === 'undefined') return;
      const wanted = family.slice('custom:'.length);
      if ([...fontFaces.values()].some((f) => f.family.replace(/^"|"$/g, '') === wanted)) return;
      const font = (await list()).find((f) => f.family === wanted);
      if (font) await load(font);
    },
  };
}
