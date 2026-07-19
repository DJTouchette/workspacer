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

  var api = {
    ready: ready,
    connected: false,
    token: token,
    url: url,
    settings: initialSettings(),
    call: call,
    publish: publish,
    on: on,
    onSettings: onSettings,
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
      if (!readyResolved) {
        readyResolved = true;
        readyResolve();
      }
    };
    ws.onmessage = function (e) {
      onMessage(e.data);
    };
    ws.onclose = function () {
      api.connected = false;
      rejectPending("workspacer: connection closed");
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
