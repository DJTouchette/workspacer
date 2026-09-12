export const FONT_EXT = /\.(ttf|otf|woff2?)$/i;
/** Same family identity on native CSS and browser FontFace transports. */
export function customFontFamily(file: string): string {
  return file.replace(FONT_EXT, '').replace(/\[[^\]]*\]/g, '')
    .replace(/[-_. ]?(VariableFont[^.]*|Variable|Regular|VF)$/i, '')
    .replace(/[-_.]+/g, ' ').trim() || file;
}
