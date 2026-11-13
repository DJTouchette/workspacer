#!/usr/bin/env python3
"""Regenerate landing/build-plugin.md from landing/build-plugin.html.

The HTML page is the source of truth; the Markdown twin is the same text for
LLMs. It refuses markup it does not know how to convert rather than dropping
it, so a new kind of block in the page fails here instead of going missing.

Usage: gen-build-plugin-md.py build-plugin.html > build-plugin.md
(or `make landing-plugin-md` from the repository root)
"""
import html, re, sys
from html.parser import HTMLParser

src = open(sys.argv[1], encoding="utf-8").read()
main = src[src.index("<main>") + 6 : src.index("</main>")]

def code_span(text):
    text = html.unescape(text)
    return f"`` {text} ``" if "`" in text else f"`{text}`"

def inline(fragment):
    """Inline HTML -> Markdown. Code first, so its contents are never re-read."""
    spans = []
    def stash(m):
        spans.append(code_span(m.group(1)))
        return f"\x00{len(spans) - 1}\x00"
    out = re.sub(r"<code>(.*?)</code>", stash, fragment, flags=re.S)
    def link(m):
        href, text = m.group(1), m.group(2)
        return text if href.startswith("#") else f"[{text}]({href})"
    out = re.sub(r'<a\s+href="([^"]*)"[^>]*>(.*?)</a>', link, out, flags=re.S)
    out = re.sub(r"<strong>(.*?)</strong>", r"**\1**", out, flags=re.S)
    out = re.sub(r"<em>(.*?)</em>", r"*\1*", out, flags=re.S)
    assert "<" not in re.sub(r"\x00\d+\x00", "", out).replace("&lt;", ""), out[:200]
    out = html.unescape(out)
    out = re.sub(r"\s*\n\s*", " ", out).strip()
    return re.sub(r"\x00(\d+)\x00", lambda m: spans[int(m.group(1))], out)

def fence(body):
    body = html.unescape(body).strip("\n")
    first = body.lstrip()[:1]
    if first in "{\"[" :
        lang = "json"
    elif re.search(r"\bconst \b|window\.|ws\.send|^//|=>|require\(", body, re.M):
        lang = "js"
    else:
        lang = ""
    return f"```{lang}\n{body}\n```"

blocks = ["# How to build a work{spacer} plugin", "> HTML version: build-plugin.html"]
token = re.compile(
    r"<h2>(?P<h2>.*?)</h2>|<h3>(?P<h3>.*?)</h3>|<p>(?P<p>.*?)</p>|<ul>(?P<ul>.*?)</ul>"
    r"|<pre><code>(?P<pre>.*?)</code></pre>|<div class=\"callout\">(?P<callout>.*?)</div>"
    r"|<div class=\"foot\">.*?</div>|</?section[^>]*>",
    re.S,
)
pos = 0
for m in token.finditer(main):
    gap = main[pos : m.start()].strip()
    assert not gap, f"unconverted markup: {gap[:120]!r}"
    pos = m.end()
    if m.group("h2") is not None:
        title = inline(m.group("h2"))
        blocks.append("## " + title[:1].upper() + title[1:])
    elif m.group("h3") is not None:
        blocks.append("### " + inline(m.group("h3")))
    elif m.group("p") is not None:
        blocks.append(inline(m.group("p")))
    elif m.group("ul") is not None:
        items = re.findall(r"<li>(.*?)</li>", m.group("ul"), re.S)
        blocks.append("\n".join("- " + inline(item) for item in items))
    elif m.group("pre") is not None:
        blocks.append(fence(m.group("pre")))
    elif m.group("callout") is not None:
        blocks.append("> " + inline(m.group("callout")))
assert not main[pos:].strip(), main[pos:][:120]
sys.stdout.write("\n\n".join(blocks) + "\n")
