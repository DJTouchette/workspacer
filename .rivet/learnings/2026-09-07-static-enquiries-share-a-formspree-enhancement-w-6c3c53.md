---
title: Static enquiries share a Formspree enhancement with a native POST fallback
date: 2026-09-07
confidence: high
related_paths:
  - landing/contact.js
  - landing/contact.css
  - landing/enterprise.html
promoted: false
---

# Static enquiries share a Formspree enhancement with a native POST fallback

## Observation
landing/contact.js enhances the inline pilot form and shared enquiry dialogs using @formspree/ajax 1.1.5. Form markup retains action/method POST; links fall back to enterprise.html#pilot if JS or dialog support is absent. SDK success fixtures need a next string (not only ok:true), and notification recipients are controlled by the Formspree dashboard, not the HTML endpoint.

## Recommendation
Keep field names consistent in the inline form and dialog template. Intercept endpoint requests for browser tests; success rendering does not verify actual inbox delivery.
