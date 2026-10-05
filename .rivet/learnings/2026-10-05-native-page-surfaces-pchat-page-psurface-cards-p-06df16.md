---
title: Native page surfaces: p.chat page, p.surface cards, p.base only for chrome
date: 2026-10-05
promoted: false
---

# Native page surfaces: p.chat page, p.surface cards, p.base only for chrome

## Observation
In apps/native the shell paints every page in p.chat (navigation.rs shell). Page content uses chrome::card (panel_radius/surface/border); floating docks add chrome::floating_shadow. A p.base strip on a page reads as an off-theme square band (bug #22, New Agent footer in launch.rs). Sticky page footers should be an inset chrome::card using the scroll area's own gutter so edges align with the cards. Note keycap() uses p.surface, so on a surface panel it loses its chip (the chat composer has the same look).
