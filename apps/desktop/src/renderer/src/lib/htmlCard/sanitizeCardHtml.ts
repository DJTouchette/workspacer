/** DOMPurify handles HTML parser mutations. Our allowlist removes navigation,
 * embedded documents and executable content; CSP separately blocks resources.
 * No model JavaScript is permitted: CSP does NOT block frame self-navigation.
 */
import createDOMPurify from 'dompurify';

const ALLOWED_TAGS = new Set([
  'a',
  'abbr',
  'article',
  'aside',
  'b',
  'bdi',
  'bdo',
  'blockquote',
  'br',
  'button',
  'caption',
  'cite',
  'code',
  'col',
  'colgroup',
  'dd',
  'del',
  'details',
  'dfn',
  'div',
  'dl',
  'dt',
  'em',
  'figcaption',
  'figure',
  'footer',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'header',
  'hr',
  'i',
  'img',
  'input',
  'ins',
  'kbd',
  'label',
  'li',
  'main',
  'mark',
  'meter',
  'nav',
  'ol',
  'output',
  'p',
  'pre',
  'progress',
  'q',
  's',
  'samp',
  'section',
  'small',
  'span',
  'strong',
  'sub',
  'summary',
  'sup',
  'table',
  'tbody',
  'td',
  'tfoot',
  'th',
  'thead',
  'time',
  'tr',
  'u',
  'ul',
  'var',
  'wbr',
]);

const DROPPED_SUBTREE_TAGS = new Set([
  'applet',
  'audio',
  'base',
  'canvas',
  'embed',
  'form',
  'frame',
  'frameset',
  'head',
  'iframe',
  'link',
  'map',
  'math',
  'meta',
  'noembed',
  'noframes',
  'noscript',
  'object',
  'param',
  'portal',
  'script',
  'select',
  'slot',
  'source',
  'style',
  'svg',
  'template',
  'textarea',
  'title',
  'track',
  'video',
  'xmp',
]);

const purifier = createDOMPurify(window);
const attrs = [
  'class',
  'id',
  'title',
  'dir',
  'lang',
  'role',
  'hidden',
  'style',
  'alt',
  'width',
  'height',
  'type',
  'placeholder',
  'value',
  'checked',
  'disabled',
  'readonly',
  'open',
  'start',
  'reversed',
  'colspan',
  'rowspan',
  'scope',
  'span',
  'datetime',
  'min',
  'max',
  'low',
  'high',
  'optimum',
  'for',
  'data-wks-filter',
  'data-wks-filter-item',
  'data-wks-filter-count',
  'data-wks-sort',
];

export function isSafeStyleAttr(value: string): boolean {
  return (
    !/[<>{}\\]|\/\*|\*\//.test(value) &&
    !/(?:url|image|image-set|cross-fade|element|expression)\s*\(/i.test(value)
  );
}
purifier.addHook('uponSanitizeAttribute', (node, data) => {
  if (data.attrName === 'style' && !isSafeStyleAttr(data.attrValue)) data.keepAttr = false;
  if (
    data.attrName === 'type' &&
    node.nodeName === 'INPUT' &&
    !['text', 'search', 'checkbox', 'radio'].includes(data.attrValue.toLowerCase())
  ) {
    data.attrValue = 'text';
  }
});

export interface SanitizeResult {
  html: string;
  removed: string[];
}
export function sanitizeCardHtml(bodyHtml: string): SanitizeResult {
  const html = purifier.sanitize(bodyHtml, {
    ALLOWED_TAGS: [...ALLOWED_TAGS],
    ALLOWED_ATTR: attrs,
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: true,
    FORBID_TAGS: [...DROPPED_SUBTREE_TAGS],
    FORBID_CONTENTS: [...DROPPED_SUBTREE_TAGS],
    // Even fragment hrefs on about:srcdoc can resolve against the embedding
    // document. V1 removes ALL URLs, including data images and anchor hrefs.
    FORBID_ATTR: ['href', 'src', 'srcset', 'name', 'nonce', 'form', 'formaction'],
  });
  return {
    html,
    removed: purifier.removed.map((item) =>
      'attribute' in item ? (item.attribute?.name ?? 'attribute') : 'element',
    ),
  };
}

/** CSS remains opaque stylesheet text. Removing '<' prevents raw-text shell
 * breakout. Resource-bearing CSS (including escaped url/import) is blocked by
 * CSP, not a regex pretending to parse CSS. */
export function sanitizeCardCss(css: string): string {
  return css.replace(/</g, ' ');
}
