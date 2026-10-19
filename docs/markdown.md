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

Links, images, raw HTML, block quotes and tables are retained as source text
with warnings. Complex list containers are simplified to separate paragraphs.
The importer supports this subset rather than complete CommonMark rendering.
Warnings returned by placement identify unsupported or simplified constructs.

Inputs are limited to 16 MiB, 10,000 paragraphs, 100,000 character runs and
32 levels of list/delimiter nesting. Delimiter processing also has a finite work
budget; oversized inputs return an error instead of importing partial content.

For example, `place.styles {"path": "article.md"}` lists imported styles before
`file.place {"path": "article.md", "styleMap": {"Heading 1": "Headline"}}` places
the text using the document's Headline style.
