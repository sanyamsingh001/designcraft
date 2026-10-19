# Markdown placement

File > Place accepts `.md` and `.markdown` files on desktop and in the browser.
The same engine path is available through `file.place` and the CLI. Placement
imports a copy of the content; it does not establish a live synchronization link.

Supported syntax includes ATX and setext headings, paragraphs, hard line breaks,
nested bullet and ordered lists, emphasis, strong emphasis, strikethrough, inline
code, and fenced or indented code blocks. Generated paragraph and character
styles appear in `place.styles` and use the existing `styleMap`, `styleConflicts`
and `removeStyles` options. Heading sizes and list defaults belong to named
styles so they can be mapped to document styles.

Inline emphasis, strong emphasis, double-tilde strikethrough, escapes and code
spans use pulldown-cmark's maintained grammar. Entities retain their source
spelling, and NUL characters are retained, including inside code spans.
Double tildes follow GFM punctuation flanking: `a~~!b~~c` becomes `a!bc`
with `!b` struck, and `a~~b!~~c` becomes `ab!c` with `b!` struck.
Single and longer tilde runs remain literal.

Block classification and source-retention rules remain application policy.
Links, images, raw HTML, block quotes and tables are retained as source text
with warnings. A paragraph containing a recognized link, image or inline HTML
stays wholly literal, including any supported-looking emphasis. Reference
definitions and their multiline titles remain visible. Complex list containers
are simplified to separate paragraphs. This subset is not complete CommonMark
rendering. Warnings identify unsupported or simplified constructs.

Inputs are limited to 16 MiB, 10,000 paragraphs, 100,000 character runs and
32 levels of list or inline formatting nesting. Unmatched delimiters remain
literal and do not consume formatting depth. A finite work budget limits adapter
preprocessing and event handling; input size also bounds the library's eager
parsing input. Oversized inputs return an error instead of importing partial
content.

For example, `place.styles {"path": "article.md"}` lists imported styles before
`file.place {"path": "article.md", "styleMap": {"Heading 1": "Headline"}}` places
the text using the document's Headline style.
