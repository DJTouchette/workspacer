---
title: notifications.post accepted a pane click target that the OS-notification click silently dropped
date: 2026-08-24
confidence: high
suggested_doc: mcp-tool-facade
promoted: false
---

# notifications.post accepted a pane click target that the OS-notification click silently dropped

## Observation
Two independent gaps in the notification click path, both closed 2026-08-24. (1) The facade's `notify` tool input (notifyIn, cmd/mcp/main.go) carried only title+body while the capability behind it accepted sessionId/paneType/url/level/key/silent/inAppOnly — so an agent could not post a click-through notification at all. (2) hubCapabilities' own OS notification branched on sessionId → url → focusWindow and never handled paneType, so even a pane target posted by a plugin went nowhere. Separately, a notification click could not reach a Settings SECTION: App.tsx's onOpenPane called handleAddTab only, opening Settings wherever it was last.</observation>
<parameter name="impact">The one shipping propose-then-arm flow (jobs.propose) told the user "review it in Settings → Jobs" in prose and made them navigate by hand.</impact>
<parameter name="recommendation">A new InAppNotification field must be added in six places or it is silently dropped: notifyIn (Go tool schema), hubCapabilities' notifications.post destructure AND its postInApp call, InAppNotification (ipcTypes.ts), NotificationInput + normalizeNotification (notificationStore.ts — an explicit allowlist), and the consumer in NotificationsContext.activate. postInApp itself spreads, so it is not a gate. The OS-notification click now routes agent/pane targets through agentNotifier.activateInRenderer while `url` stays on the host for its scheme check.</recommendation>
<parameter name="related_paths">["services/hub/cmd/mcp/main.go", "apps/desktop/src/main/services/hubCapabilities.ts", "apps/desktop/src/renderer/src/lib/notificationStore.ts"]
