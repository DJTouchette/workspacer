---
title: GPUI native: Ctrl+Enter in any Workspace Input reaches SendMessage; scope picker keys and consume Enter
date: 2026-10-05
author: claude
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/features.rs
promoted: false
---

# GPUI native: Ctrl+Enter in any Workspace Input reaches SendMessage; scope picker keys and consume Enter

## Observation
SendMessage is bound to ctrl/cmd-enter in 'Workspace > Input', and Workspace::send() always reads the composer draft, so Ctrl+Enter in any other single-line Input under the workspace (the old question answer fields) sent the composer draft as a chat message. GPUI ranks bindings by matched context depth and breaks ties by registration order, so a 'QuestionPicker > Input' binding only wins if bind_keys registers it after the Workspace ones. Separately, gpui-component single-line Input's Enter action calls cx.propagate(); if a handler moves focus, the unhandled keystroke's key_char '\n' is then delivered as text to the newly focused Input (panics in tests: 'text argument should not contain newlines'). Bind Enter to an app action in the scoped context instead of subscribing to InputEvent::PressEnter. Test keystrokes are key-down only, while GPUI keyboard-click on focusable divs fires on key-up; send a KeyUpEvent in tests. ScrollHandle::scroll_to_item indexes direct children, and gpui-component vertical_scrollbar() appends a child, so call it after .children().

## Impact
New inputs or pickers in the native chat can silently send the user's draft, or corrupt the next field, from ordinary keyboard use.

## Recommendation
Give every non-composer input region its own key_context with Ctrl/Cmd+Enter and Enter bindings registered after the Workspace bindings; consume Enter via an action, not PressEnter. See ui/features.rs render_questions and ui.rs bind_keys.
