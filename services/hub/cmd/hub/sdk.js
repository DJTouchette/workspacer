// workspacer plugin SDK — host-served, auto-injected into every plugin webview
// at /plugins/ui/<id>/. Plain ES, no build step; works under contextIsolation
// with no preload. It owns the bus WebSocket so a plugin only touches
// window.workspacer instead of hand-rolling call/publish/subscribe + reconnect.
(function () {
  "use strict";

  var params = new URLSearchParams(location.search);
  var token = params.get("busToken") || "";

  // Build the bus URL from the page origin; fall back to the loopback default
  // when the document has no host (e.g. file:// or an about:blank shell).
  function busURL() {
    var host = location.host;
    if (host) {
      var proto = location.protocol === "https:" ? "wss:" : "ws:";
      return proto + "//" + host + "/bus?token=" + encodeURIComponent(token);
    }
    return "ws://127.0.0.1:7895/bus?token=" + encodeURIComponent(token);
  }

  var url = busURL();
  var ws = null;
  var nextId = 1;
  var pending = new Map(); // call id (string) -> { resolve, reject }
  var handlers = new Map(); // event type -> Set<fn(data, event)>
  var settingsHandlers = new Set(); // fn(settings)
  var statusHandlers = new Set(); // fn(connected)
  var backoff = 500; // ms, doubles to a cap on each failed (re)connect
  var readyResolved = false;
  var readyResolve;
  var ready = new Promise(function (res) {
    readyResolve = res;
  });

  function initialSettings() {
    var s = window.__WKS_SETTINGS__;
    return s && typeof s === "object" ? s : {};
  }

  var providers = new Map(); // method -> async handler(params) => result

  var api = {
    ready: ready,
    connected: false,
    token: token,
    url: url,
    settings: initialSettings(),
    call: call,
    publish: publish,
    provide: provide,
    on: on,
    onSettings: onSettings,
    onStatus: onStatus,
  };

  function pluginId() {
    return window.__WKS_PLUGIN_ID__ || "plugin";
  }

  function send(obj) {
    if (!ws || ws.readyState !== 1 /* OPEN */) return false;
    try {
      ws.send(JSON.stringify(obj));
      return true;
    } catch (e) {
      return false;
    }
  }

  // call(method, params) -> Promise. Resolves with the reply `result`, rejects
  // with an Error on an error reply or if the socket closes while in flight.
  // The bus keys correlation ids as strings, so we send string ids.
  function call(method, params) {
    return new Promise(function (resolve, reject) {
      var id = String(nextId++);
      pending.set(id, { resolve: resolve, reject: reject });
      if (!send({ op: "call", id: id, method: method, params: params })) {
        pending.delete(id);
        reject(new Error("workspacer: not connected"));
      }
    });
  }

  function publish(type, data) {
    send({ op: "publish", event: { type: type, source: pluginId(), data: data } });
  }

  // provide(method, handler) — answer a bus method this plugin declares in its
  // manifest `provides` (and, through a manifest `tools` entry, expose to
  // agents as an MCP tool). handler(params) may return a value or a Promise;
  // a throw/rejection becomes the caller's error reply. Registration is sent
  // now and re-sent on every reconnect; the hub silently drops methods the
  // plugin's consented grant doesn't cover (watch the `registered` ack in the
  // hub log). There is no unregister op — a provider slot frees when the
  // connection drops.
  function provide(method, handler) {
    if (typeof method !== "string" || !method || typeof handler !== "function") {
      throw new Error("workspacer.provide(method, handler): bad arguments");
    }
    providers.set(method, handler);
    if (api.connected) {
      send({ op: "register", methods: [method] });
    }
  }

  function registerProviders() {
    if (!providers.size) return;
    send({ op: "register", methods: Array.from(providers.keys()) });
  }

  function answerCall(msg) {
    var handler = providers.get(msg.method);
    if (!handler) {
      send({ op: "error", id: msg.id, error: "no handler for " + msg.method });
      return;
    }
    Promise.resolve()
      .then(function () {
        return handler(msg.params);
      })
      .then(function (result) {
        send({ op: "result", id: msg.id, result: result === undefined ? null : result });
      })
      .catch(function (err) {
        send({ op: "error", id: msg.id, error: String((err && err.message) || err) });
      });
  }

  // on(type, handler) -> off(). "*" receives every inbound event. handler gets
  // (data, event). Returns an idempotent unsubscribe.
  function on(type, handler) {
    if (typeof handler !== "function") return function () {};
    var set = handlers.get(type);
    if (!set) {
      set = new Set();
      handlers.set(type, set);
    }
    set.add(handler);
    return function off() {
      var s = handlers.get(type);
      if (s) {
        s.delete(handler);
        if (!s.size) handlers.delete(type);
      }
    };
  }

  // onSettings(handler) -> off(). Fires with the new merged settings object on a
  // settings bus event; also refreshes workspacer.settings.
  function onSettings(handler) {
    if (typeof handler !== "function") return function () {};
    settingsHandlers.add(handler);
    return function off() {
      settingsHandlers.delete(handler);
    };
  }

  // onStatus(handler) -> off(). Fires handler(connected) with true on every ws
  // open and false on every ws close (including reconnect cycles), so a plugin
  // can reflect live connect/disconnect. Kept separate from on(type, ...) so the
  // bus event-type space isn't polluted. Returns an idempotent unsubscribe.
  function onStatus(handler) {
    if (typeof handler !== "function") return function () {};
    statusHandlers.add(handler);
    return function off() {
      statusHandlers.delete(handler);
    };
  }

  function fireStatus(connected) {
    statusHandlers.forEach(function (h) {
      try {
        h(connected);
      } catch (e) {
        /* a broken handler must not kill the status fan-out */
      }
    });
  }

  function fire(type, ev) {
    var set = handlers.get(type);
    if (!set) return;
    set.forEach(function (h) {
      try {
        h(ev.data, ev);
      } catch (e) {
        /* a broken handler must not kill the dispatch loop */
      }
    });
  }

  // The hub publishes plugin.settings.changed as { id, values }; a plain
  // wks-settings event may carry the values object directly.
  function extractSettings(data) {
    if (!data || typeof data !== "object") return null;
    if (data.values && typeof data.values === "object") return data.values;
    return data;
  }

  function dispatchEvent(ev) {
    if (!ev || typeof ev !== "object") return;
    var type = ev.type;
    if (type === "plugin.settings.changed" || type === "wks-settings") {
      var next = extractSettings(ev.data);
      if (next && typeof next === "object") {
        api.settings = next;
        window.__WKS_SETTINGS__ = next;
        settingsHandlers.forEach(function (h) {
          try {
            h(next);
          } catch (e) {
            /* ignore */
          }
        });
      }
    }
    fire(type, ev);
    fire("*", ev);
  }

  function onMessage(raw) {
    var msg;
    try {
      msg = JSON.parse(raw);
    } catch (e) {
      return;
    }
    if (!msg || typeof msg !== "object") return;
    switch (msg.op) {
      case "result": {
        var p = pending.get(msg.id);
        if (p) {
          pending.delete(msg.id);
          p.resolve(msg.result);
        }
        break;
      }
      case "error": {
        var pe = pending.get(msg.id);
        if (pe) {
          pending.delete(msg.id);
          pe.reject(new Error(msg.error || "workspacer: call failed"));
        }
        break;
      }
      case "event":
        dispatchEvent(msg.event);
        break;
      case "call":
        // Inbound RPC: the hub routed a caller's capability call to us because
        // we registered the method. Reply on the SAME id (the router's global
        // correlation id).
        answerCall(msg);
        break;
    }
  }

  function rejectPending(reason) {
    pending.forEach(function (p) {
      try {
        p.reject(new Error(reason));
      } catch (e) {
        /* ignore */
      }
    });
    pending.clear();
  }

  function scheduleReconnect() {
    var delay = backoff;
    backoff = Math.min(backoff * 2, 8000);
    setTimeout(connect, delay);
  }

  function connect() {
    try {
      ws = new WebSocket(url);
    } catch (e) {
      scheduleReconnect();
      return;
    }
    ws.onopen = function () {
      api.connected = true;
      backoff = 500; // reset on a clean open
      send({ op: "subscribe", topics: ["*"] });
      registerProviders();
      if (!readyResolved) {
        readyResolved = true;
        readyResolve();
      }
      fireStatus(true);
    };
    ws.onmessage = function (e) {
      onMessage(e.data);
    };
    ws.onclose = function () {
      api.connected = false;
      rejectPending("workspacer: connection closed");
      fireStatus(false);
      scheduleReconnect();
    };
    ws.onerror = function () {
      // A failed connection also fires onclose, which owns reconnect; keep this
      // a no-op so we don't schedule twice.
    };
  }

  window.workspacer = api;
  connect();
})();
