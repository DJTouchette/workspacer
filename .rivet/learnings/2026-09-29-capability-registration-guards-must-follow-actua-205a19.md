---
title: Capability registration guards must follow actual imported method lists
date: 2026-09-29
promoted: false
---

# Capability registration guards must follow actual imported method lists

## Observation
The retained desktop registers native desktop services through a for-of loop spreading desktopServices.generated ownerMethods and assetMethods, so a literal-call regex silently omits those methods. The portable capabilitySource scanner resolves literal const arrays, property access and actual default-imported generated source, while rejecting computed registration expressions. Rust scans every production module and explicitly accounts for plugins.tools readiness replacement and fleet.dispatchTargets paired-service replacement; provider/core implementation co-location does not erase captured hub ownership disjointness.
