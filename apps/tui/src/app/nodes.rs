//! The remote-node surface's app half: seeding the registry, the overlay's
//! cursor, and the two-step wake.
//!
//! **The two-step wake is the point of this file.** `nodes.wake` starts a
//! billable machine and the hub deliberately has no verb to stop one, so an
//! accidental wake cannot be undone from inside the app at all. The desktop
//! and `/m` both shipped a confirmation step for that reason; a vim-modal TUI
//! needs it more, not less, because a stray keystroke in normal mode is its
//! native failure. So [`App::request_wake`] only ARMS the action — it fires
//! nothing — and [`App::confirm_wake`] is the only path to the bus.
//!
//! Everything about which nodes exist and what state they are in comes from
//! the hub, never from inference here: see [`crate::nodes`].

use super::*;
use crate::nodes::{NodeRegistry, NodeView};

impl App {
    /// May this connection wake a node?
    ///
    /// `nodes.wake` is host-authority-only — refused twice hub-side, and absent
    /// from BOTH scoped tiers' allowlists. The bus `hello` frame names the tier
    /// we authenticated as, so we know before offering anything. Absent (an
    /// unknown greeting, a plugin token, a hub that never greeted) reads as NO:
    /// for a control that spends money, the safe default is the honest one, and
    /// the overlay prints the reason rather than silently hiding the row.
    pub fn can_wake_nodes(&self) -> bool {
        self.bus_scope.as_deref() == Some("operator")
    }

    /// (Re)seed the node registry from `nodes.list`. Fired on every bus
    /// (re)connect and on opening the overlay; never polled — the hub
    /// publishes `node.state_changed` only on a real change, so every event it
    /// sends is worth acting on and no timer is needed between them.
    pub(in crate::app) fn seed_nodes(&self) {
        let Some(bus) = self.bus.clone() else { return };
        tokio::spawn(crate::nodes::seed_nodes(bus, self.tx.clone()));
    }

    /// Adopt a `nodes.list` roster, or the feature-absent answer (`None`).
    pub(in crate::app) fn set_nodes(&mut self, nodes: Option<Vec<NodeView>>) {
        match nodes {
            Some(list) => match self.nodes.as_mut() {
                Some(reg) => reg.seed(list),
                None => self.nodes = Some(NodeRegistry::new(list)),
            },
            None => self.nodes = None,
        }
        self.clamp_nodes_cursor();
    }

    /// Patch one node from `node.state_changed` (or a wake's own answer). The
    /// hub has spoken about this node, so our optimistic pending flag and any
    /// stale wake error for it are both superseded.
    pub(in crate::app) fn apply_node_change(&mut self, node: NodeView) {
        let id = node.id.clone();
        match self.nodes.as_mut() {
            Some(reg) => reg.apply_change(node),
            None => self.nodes = Some(NodeRegistry::new(vec![node])),
        }
        if let Some(v) = self.nodes_view.as_mut() {
            v.pending.remove(&id);
            v.errors.remove(&id);
            // A confirmation left armed for a node the hub just moved is stale;
            // never let it apply to whatever the row has become.
            if v.confirm.as_deref() == Some(id.as_str()) {
                v.confirm = None;
            }
        }
        self.clamp_nodes_cursor();
    }

    // ── the overlay ─────────────────────────────────────────────────────────

    /// Open the remote-node overlay and re-seed it.
    ///
    /// Unlike the dashboard line (which stays silent when there is nothing to
    /// report, mirroring the desktop strip), an explicitly requested overlay
    /// always opens and answers — including with "this hub has no remote
    /// nodes", which is the true answer for almost every install.
    pub(in crate::app) fn open_nodes(&mut self) {
        if self.bus.is_none() {
            self.set_toast("remote nodes need the hub bus (this session is claudemon-direct)");
            return;
        }
        self.nodes_view = Some(NodesState {
            selected: 0,
            confirm: None,
            pending: std::collections::HashSet::new(),
            errors: HashMap::new(),
        });
        self.seed_nodes();
    }

    pub(in crate::app) fn close_nodes(&mut self) {
        self.nodes_view = None;
    }

    /// The registry rows the overlay renders, in the hub's own order.
    pub fn node_rows(&self) -> &[NodeView] {
        self.nodes.as_ref().map(NodeRegistry::nodes).unwrap_or(&[])
    }

    /// The node under the overlay's cursor.
    pub fn selected_node(&self) -> Option<&NodeView> {
        let idx = self.nodes_view.as_ref()?.selected;
        self.node_rows().get(idx)
    }

    pub(in crate::app) fn nodes_select_next(&mut self) {
        let last = self.node_rows().len().saturating_sub(1);
        if let Some(v) = self.nodes_view.as_mut() {
            v.selected = v.selected.saturating_add(1).min(last);
        }
    }

    pub(in crate::app) fn nodes_select_prev(&mut self) {
        if let Some(v) = self.nodes_view.as_mut() {
            v.selected = v.selected.saturating_sub(1);
        }
    }

    /// Keep the cursor inside a roster that may have shrunk under it.
    fn clamp_nodes_cursor(&mut self) {
        let last = self.node_rows().len().saturating_sub(1);
        if let Some(v) = self.nodes_view.as_mut() {
            v.selected = v.selected.min(last);
        }
    }

    // ── the two-step wake ───────────────────────────────────────────────────

    /// `w`: ARM a wake. This fires nothing.
    ///
    /// It refuses outright wherever [`crate::nodes::wake_affordance`] says the
    /// hub would — a view/triage tier, a node with no credential on this hub,
    /// one already starting, one already up — and toasts that reason instead of
    /// arming a confirmation that could only end in a refusal.
    pub(in crate::app) fn request_wake(&mut self) {
        let Some(node) = self.selected_node().cloned() else {
            self.set_toast("no node selected");
            return;
        };
        let pending = self
            .nodes_view
            .as_ref()
            .is_some_and(|v| v.pending.contains(&node.id));
        let affordance = crate::nodes::wake_affordance(&node, self.can_wake_nodes(), pending);
        if !affordance.enabled {
            // `reason` is empty only for an already-connected node, which needs
            // no explanation beyond the state already on its row.
            if affordance.reason.is_empty() {
                self.set_toast(format!("{} is already connected", node.label));
            } else {
                self.set_toast(affordance.reason);
            }
            return;
        }
        if let Some(v) = self.nodes_view.as_mut() {
            v.confirm = Some(node.id);
        }
    }

    /// Anything that is not the confirm key: stand down, spend nothing.
    pub(in crate::app) fn cancel_wake(&mut self) {
        if let Some(v) = self.nodes_view.as_mut() {
            v.confirm = None;
        }
    }

    /// The confirm key. THIS is the one path that spends money.
    ///
    /// The affordance is re-checked rather than trusted from `request_wake`: a
    /// `node.state_changed` can land between arming and confirming, and the
    /// answer to "may this be woken" is allowed to have changed in that window.
    pub(in crate::app) fn confirm_wake(&mut self) {
        let Some(id) = self.nodes_view.as_mut().and_then(|v| v.confirm.take()) else {
            return;
        };
        let Some(node) = self.nodes.as_ref().and_then(|r| r.get(&id)).cloned() else {
            self.set_toast("this machine is no longer in the registry.");
            return;
        };
        let pending = self
            .nodes_view
            .as_ref()
            .is_some_and(|v| v.pending.contains(&id));
        if !crate::nodes::wake_affordance(&node, self.can_wake_nodes(), pending).enabled {
            self.set_toast("that machine can no longer be woken from here");
            return;
        }
        let Some(bus) = self.bus.clone() else { return };
        if let Some(v) = self.nodes_view.as_mut() {
            v.pending.insert(id.clone());
            v.errors.remove(&id);
        }
        self.set_toast(format!(
            "waking {} — {}",
            node.label,
            crate::nodes::WAKE_COST_NOTE
        ));
        tokio::spawn(crate::nodes::wake_node(bus, id, self.tx.clone()));
    }

    /// The wake's own answer: the hub's `NodeView` (normally `waking`), or a
    /// rendered failure. A refusal is recorded on the row AND toasted — it is
    /// the only notice anyone gets that the machine did not start.
    pub(in crate::app) fn apply_node_wake(
        &mut self,
        id: String,
        node: Option<NodeView>,
        error: Option<String>,
    ) {
        if let Some(v) = self.nodes_view.as_mut() {
            v.pending.remove(&id);
        }
        if let Some(node) = node {
            self.apply_node_change(node);
        }
        if let Some(err) = error {
            if let Some(v) = self.nodes_view.as_mut() {
                v.errors.insert(id, err.clone());
            }
            self.set_toast(err);
        }
    }
}
