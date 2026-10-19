# Designcraft source recovery — UNTESTED

This branch indexes source backups made before the requested workspace reset. These are recovery copies, not accepted feature publications. No CI was run for these backup commits.

The [machine-readable manifest](recovery-manifest.json) maps 393 verified source refs: 95 snapshots/aliases, 101 exact original commits, 194 additional staged trees, and three separate conflict-stage views. It covers all 93 physical checkouts surviving at intake, 495 registered HEADs, and 494 surviving worktree indexes.

## Priority source

- [Root staged source](https://github.com/sanyamsingh001/designcraft/tree/recovery/untested-design-20261019-1854/root-staged)
- [Root working source](https://github.com/sanyamsingh001/designcraft/tree/recovery/untested-design-20261019-1854/root)
- [PR1305 shared-options reliability source, dc8ec21](https://github.com/sanyamsingh001/designcraft/tree/recovery/untested-design-20261019-1854/parity-pr1305-shared-options-20261019-58ac19d4)
- [Markdown source, 83c0bebb](https://github.com/sanyamsingh001/designcraft/tree/recovery/untested-design-20261019-1854/parity-pr462-markdown-443-shared-options-20261019-58ac19d4)
- [Complete 577-file validation source](https://github.com/sanyamsingh001/designcraft/tree/recovery/untested-design-20261019-1854/parity-validation-dc8-complete)

## Limits

The private evidence directory disappeared before backup. Source proposals existing only there cannot be claimed recovered. For the 402 registered directories already absent at intake, recorded commits and surviving staged indexes are saved, but missing unstaged/untracked bytes remain unknown. Build outputs, logs, private captures and credentials are excluded.

Conflicting variants remain separate. The unmerged index is represented by three labeled stage views, without resolution. Existing published branches were not updated. See the manifest for exact source trees and mappings.
