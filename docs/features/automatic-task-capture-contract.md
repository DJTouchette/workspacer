# Manager request admission boundary

Implementation baseline: `6d9a6b86414d942a5a9ece8f885153ca56176c0e`
(the accepted compact Inspector and authenticated task references), fast-forwarded
from `ac818c86` in the isolated automatic-task branch. The later Windows/integration
commits are not included in this baseline.

## Transport facts verified in the implementation checkout

- `main/ipc.ts` handles `claude:message` with only session ID and text. Remote
  sessions forward through `agents.sendMessage`; local sessions use
  `ClaudemonSessionClient.message`.
- `ClaudemonSessionClient.message` mints an in-memory frame ID **after** the
  handoff hold check. `messageDirect` posts only `{text}` and discards the
  successful response body, including the daemon's `queued` flag.
- `daemon/api.rs::MessagePayload` contains only `text`. `post_message` reports
  successful queue admission, not provider consumption.
- `SessionStore::native_submit_message` forwards managed input to an
  `UnboundedSender<String>`. The pending queue's drain explicitly maps every
  `PendingMessage` to its text. Neither route carries host request identity or
  separate trusted context to the provider.
- `ManagerReplacementState.holdMessage` allocates another delivery ID and
  persists text. `ManagerReplacementService.deliver` forwards only target and
  text, appends a host correction into that text, and treats missing
  acknowledgements as uncertain. Its explicit duplicate-risk retry creates a
  new delivery ID; that ID therefore cannot be the logical user request ID.

An HTTP 200 is evidence of admission only. A transcript UUID, text digest,
timestamp, FIFO optimistic bubble, or replacement delivery ID cannot establish
the identity of a logical request across all these boundaries.

## Contract boundary needing a decision

A metadata-only desktop ledger can preserve a request ID and admission status,
but cannot attach that ID to the actual provider input on the existing text-only
transport. Publishing metadata through MCP alone does not identify which of
several queued or identical messages the manager has actually received.

Two implementable contracts are:

1. Make a desktop-owned, authenticated MCP request inbox the authoritative source
   of actionable user content. Retain bounded original request content while
   unresolved, separate it from host identity/delivery metadata in tool results,
   and interpret daemon admission as eligibility for inbox resolution even if
   the matching chat input remains queued. Resolution no longer needs to match
   a provider transcript turn. This changes the source-of-work contract and
   content-retention requirement; it is not merely a metadata ledger.
2. Add an explicit structured input envelope to daemon admission and provider
   input channels, with capability negotiation, durable logical request IDs,
   delivery-attempt IDs, and a distinct trusted context carrier. Preserve it
   through queue drain and replacement replay. Enable capture only for provider
   transports that can preserve that boundary, and report unavailable elsewhere.

Neither choice may append an authoritative request ID to untrusted user text,
infer identity from transcript matching, automatically replay ambiguous sends,
or make request resolution grant execution/publishing authority.

No feature behavior is enabled by this document. No local execution or hosted
CI has been performed for this contract investigation.
