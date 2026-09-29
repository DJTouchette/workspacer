---
title: Portable capability fixtures must use fully qualified native paths
date: 2026-09-29
promoted: false
---

# Portable capability fixtures must use fully qualified native paths

## Observation
The Windows containment job correctly rejected Unix-only /etc and /etc/shadow fixtures as nonabsolute; Git canonical operands also use path.relative platform separators. Use isolated real sibling directories for caller-chosen projects/out-of-repo operands, and path.join for expected canonical Git arguments while preserving slash-spelled input/traversal cases. Keep exact outside-repo errors and no-operation assertions; do not broaden production confinement or accept arbitrary error categories.
