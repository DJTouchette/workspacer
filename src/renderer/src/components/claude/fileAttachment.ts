// ── File Attachment Helpers ──

export const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico', 'tiff']);
export const PDF_EXTS = new Set(['pdf']);

export interface AttachedFile {
  path: string;
  label: string; // "Image" | "PDF" | "File"
  name: string;  // basename
}

export function classifyFile(filePath: string): AttachedFile {
  const name = filePath.split(/[/\\]/).pop() ?? filePath;
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  const label = IMAGE_EXTS.has(ext) ? 'Image' : PDF_EXTS.has(ext) ? 'PDF' : 'File';
  return { path: filePath, label, name };
}

export function buildPromptPrefix(files: AttachedFile[]): string {
  return files.map(f => `[${f.label}: ${f.path}]`).join(' ') + ' ';
}

/** Extract file paths from a drop or paste event */
export function extractFilePaths(dataTransfer: DataTransfer): string[] {
  const paths: string[] = [];
  if (dataTransfer.files) {
    for (let i = 0; i < dataTransfer.files.length; i++) {
      const f = dataTransfer.files[i] as File & { path?: string };
      if (f.path) paths.push(f.path);
    }
  }
  return paths;
}
