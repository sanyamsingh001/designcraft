# Markdown placement

File > Place accepts `.md` and `.markdown` files on desktop and in the browser.
The same engine path is available through `file.place` and the CLI. Placement
imports a copy by default. For files on disk, optional `link: true` or the
`linkTextFiles` preference keeps a file link for explicit Update Link or Relink
operations; it does not synchronize changes automatically.

Supported syntax includes ATX and setext headings, paragraphs, hard line breaks,
nested bullet and ordered lists, emphasis, strong emphasis, strikethrough, inline
code, and fenced or indented code blocks. Generated paragraph and character
styles appear in `place.styles` and use the existing `styleMap`, `styleConflicts`
and `removeStyles` options. Heading sizes and list defaults belong to named
styles so they can be mapped to document styles.

Block boundaries, headings, lists, code blocks, inline formatting, reference
definitions and GFM table recognition use pulldown-cmark 0.13.4. The application
maps supported content to its named styles. Entities retain their source
spelling, and NUL characters are retained, including inside code spans.
Double tildes follow the maintained parser's punctuation flanking:
`a~~!b~~c` becomes `a!bc` with `!b` struck, while `a~~b!~~c`
remains literal. Single and longer tilde runs remain literal.

Links, images, raw HTML, block quotes and tables are retained as source text
with warnings. A paragraph containing a recognized link, image or inline HTML
stays wholly literal, including any supported-looking emphasis. Recognized
reference definitions, duplicates and multiline titles remain visible in their
original rows. Definitions inside literal containers or list items do not resolve links in
other paragraphs. An ordinary unquoted line immediately after a quoted reference
definition is a lazy continuation of that quote and stays literal.

Definitions and tables follow the maintained parser's block-boundary rules:
they do not interrupt an already open paragraph. Pipes inside code-looking table
cells participate in GFM table recognition; recognized tables retain their
complete source rather than rendered cell content. Multiline inline code and
code blocks retain their code behavior. Initial list content keeps native list
formatting. Complex list items are simplified to separate paragraphs; an item
whose hidden definitions cannot be assigned to those paragraphs without losing
source is retained wholly literally with a warning.

Inputs are limited to 16 MiB, 10,000 output paragraphs, 100,000 character runs and
32 levels of list or inline formatting nesting. Unmatched delimiters remain
literal and do not consume formatting depth. Source-size admission occurs before
parser construction. Finite adapter budgets cover preprocessing, events and
output; they do not count the library's internal eager parsing work. Oversized
inputs return an error instead of importing partial content. This supported
subset is not complete CommonMark rendering.

For example, `place.styles {"path": "article.md"}` lists imported styles before
`file.place {"path": "article.md", "styleMap": {"Heading 1": "Headline"}}` places
the text using the document's Headline style.

