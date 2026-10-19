# Hyphenation data and language dispatch

The document's resolved character language selects the data, including languages
set by character, nested and GREP styles. English names and `en` locales use the
existing US English Moby dictionary and DesignCraft patterns. Spanish names and
`es` locales use Javier Bezos's patterns through `hypher` (see `NOTICE`). Russian
names and `ru` locales use the Russian data below. Other languages and `[No Language]`
have no automatic hyphenation, including Turkish, French, Ukrainian and Bulgarian.
A word containing different hyphenation languages
is left intact. Choosing a language does not infer a language from the script.
The default document language remains `English: USA`.

Russian breaks use published patterns and their explicit exceptions, without a
syllable heuristic, English inflection rules, or English fallback. Russian runs
must contain Russian letters (including precomposed ё); letters outside that
alphabet are not accepted by the Russian data. Paragraph limits, capitalization,
No Break, user exceptions and discretionary hyphens retain their existing roles.
Russian patterns require at least two letters on either side of a break, even
when a paragraph requests fewer. All returned positions are character indices;
the composer maps them to source byte offsets and complete glyph clusters.

## Russian source and license

`hyph-ru.tex` is an **unmodified** upstream file by Alexander I. Lebedev,
copyright 1999-2003, version 2003/03/10. It contains 7,021 patterns generated from
a 990,000-word list and manually corrected, plus 184 hyphenation exceptions.
DesignCraft reads the `\patterns{}` and `\hyphenation{}` blocks without executing
TeX. The runtime parser discards TeX comments; it does not alter pattern weights
or the exception entries. Regression tests check that every pattern fits the
engine's alphabet and packed key, with no silently omitted patterns.

Source repository: <https://github.com/hyphenation/tex-hyphen>

Pinned revision: `5684c0f51c0b81133db2efbe60a408b4155a3ff5`

Source file:
<https://raw.githubusercontent.com/hyphenation/tex-hyphen/5684c0f51c0b81133db2efbe60a408b4155a3ff5/hyph-utf8/tex/generic/hyph-utf8/patterns/tex/hyph-ru.tex>

SHA-256: `ce988fcc66e5e29d7474df5ab5b489d3f52a50ebccf570997f992d84221b922a`

UTF-8 source repository at that revision (not the complete original work):
<https://github.com/hyphenation/tex-hyphen/archive/5684c0f51c0b81133db2efbe60a408b4155a3ff5.tar.gz>

The original work is the four files named in the source notice: `ruhyphal.tex`,
`cyryoal.tex`, `README.ruhyphal` and `hyphen.rules`. All four are available in
<https://mirrors.ctan.org/language/hyphenation/ruhyphen.zip> (package information:
<https://ctan.org/pkg/ruhyphen>). The archive checked on 2026-10-08 has SHA-256
`60655d6dce99d11a9fbcf2e59510043276125019463146100387f767557b137e`.
The tex-hyphen repository alone does not contain those four original files.

The source declares LPPL 1.2 or later. We distribute it under LPPL 1.3c;
`LPPL-1.3c.txt` contains the license text from the SPDX license list, version
3.26.0: <https://github.com/spdx/license-list-data/blob/v3.26.0/text/LPPL-1.3c.txt>.
This data is separate from DesignCraft's MIT/Apache-2.0 Rust code. DesignCraft
keeps the upstream UTF-8 file unchanged, reads its two data blocks, discards TeX
comments at load time and preserves the pattern weights and exception entries.
Upstream authors do not provide support for DesignCraft's integration.

LPPL clause 3 covers a compiled work generated from a complete, unmodified
work; it does not itself require bundling source. Here the redistributed UTF-8
component is part of a larger original work, so do not rely solely on clause 3.
The CTAN links above supply the complete-original-work retrieval information
allowed by clause 6(d)(2). For clause 7, distribute the UTF-8 component's source
and these notices to recipients of its compiled form; keep the integration's
source available too. Merely citing an upstream repository does not replace
distribution of the source of a derived component under clause 7.

The shared helper copies this README, the unmodified UTF-8 source and its
license into a `hyphenation/` directory beside package documentation. Linux
packages share that staging tree; the source-built Flatpak installs the files
in `/app/share/doc/designcraft/hyphenation/`. Trunk copies the same three files
into its browser output, and browser ZIPs retain them. Keep that directory
available when serving or mirroring the site, including under a URL prefix.

Every GitHub release also distributes `designcraft-hyphenation-source.zip`
beside all installer, archive and browser formats. It contains these three
files, the integration source and the Rust-code licenses. Release notes link
to it and to the matching full DesignCraft source, so source retrieval does
not depend on the recipient's operating system. Mirrors must retain these
source distributions and notices alongside their binary downloads.

No upstream English patterns, GPL/LGPL files, or proprietary assets are bundled
for this integration. `ASSETS.md` also records the Russian source and license.
