(function () {
  "use strict";
  var $ = function (sel, root) { return (root || document).querySelector(sel); };
  var $$ = function (sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); };
  var csrf = document.body.getAttribute("data-csrf") || "";

  function toast(text) {
    var el = document.createElement("div");
    el.className = "toast";
    el.setAttribute("role", "alert");
    el.textContent = text;
    document.body.appendChild(el);
    setTimeout(function () { if (el.parentNode) { el.parentNode.removeChild(el); } }, 5000);
  }

  // JSON call with the CSRF header; rejects with an Error carrying the server message.
  function api(method, url, body) {
    var opts = { method: method, credentials: "same-origin", headers: { "X-CSRF-Token": csrf } };
    if (body !== undefined) {
      opts.headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    return fetch(url, opts).then(function (r) {
      return r.text().then(function (t) {
        var data = null;
        try { data = t ? JSON.parse(t) : null; } catch (e) { data = null; }
        if (!r.ok) { throw new Error((data && data.error) || ("request failed (" + r.status + ")")); }
        return data;
      });
    });
  }

  function fail(err) { toast(err && err.message ? err.message : "Something went wrong"); }
  function reload() { window.location.reload(); }

  // Delete confirmation dialog; resolves {deleteFile} or null when cancelled.
  function confirmDelete(count, hasScan) {
    var dlg = $("#delete-dialog");
    return new Promise(function (resolve) {
      if (!dlg || typeof dlg.showModal !== "function") { resolve(null); return; }
      $("#delete-text").textContent = count === 1
        ? "Delete this item? This cannot be undone."
        : "Delete " + count + " items? This cannot be undone.";
      var row = $("#delete-file-row");
      var box = $("#delete-file");
      box.checked = false;
      row.hidden = !hasScan;
      var ok = $("#delete-confirm");
      var cancel = $("#delete-cancel");
      function done(result) {
        ok.removeEventListener("click", onOk);
        cancel.removeEventListener("click", onCancel);
        dlg.removeEventListener("close", onClose);
        if (dlg.open) { dlg.close(); }
        resolve(result);
      }
      function onOk() { done({ deleteFile: hasScan && box.checked }); }
      function onCancel() { done(null); }
      function onClose() { done(null); }
      ok.addEventListener("click", onOk);
      cancel.addEventListener("click", onCancel);
      dlg.addEventListener("close", onClose);
      dlg.showModal();
    });
  }

  // ---- Files page ----
  var table = $("#files");
  var bulk = $("#bulk");
  if (bulk) {
    var countEl = $("#bulk-count");
    var form = $("#bulk-form");
    var text = $("#bulk-text");
    var actions = $("#bulk-actions");
    var pending = null;
    var rows = function () { return $$("tr[data-id]", table || document); };
    var selected = function () { return rows().filter(function (r) { return $(".sel", r).checked; }); };

    var refresh = function () {
      var sel = selected();
      rows().forEach(function (r) { r.classList.toggle("sel", $(".sel", r).checked); });
      bulk.hidden = sel.length === 0;
      countEl.textContent = sel.length + " selected";
      var all = $("#sel-all");
      if (all) { all.checked = sel.length > 0 && sel.length === rows().length; }
      if (sel.length === 0) { closeAsk(); }
    };
    var closeAsk = function () { pending = null; form.hidden = true; actions.hidden = false; text.value = ""; };

    $$("input.sel").forEach(function (cb) { cb.addEventListener("change", refresh); });
    var selAll = $("#sel-all");
    if (selAll) {
      selAll.addEventListener("change", function () {
        $$("input.sel").forEach(function (cb) { cb.checked = selAll.checked; });
        refresh();
      });
    }

    var runBulk = function (payload) {
      return api("POST", "/admin/api/media/bulk", payload).then(function (res) {
        if (res && res.failed && res.failed.length) {
          toast(res.failed.length + " failed: " + res.failed[0].error);
          if (res.ok > 0) { setTimeout(reload, 1500); }
        } else {
          reload();
        }
      }, fail);
    };
    var ids = function () { return selected().map(function (r) { return r.getAttribute("data-id"); }); };

    $$("[data-bulk]", bulk).forEach(function (btn) {
      btn.addEventListener("click", function () {
        var action = btn.getAttribute("data-bulk");
        if (btn.hasAttribute("data-ask")) {
          pending = action;
          text.setAttribute("placeholder", btn.getAttribute("data-ask"));
          text.setAttribute("aria-label", btn.getAttribute("data-ask"));
          actions.hidden = true;
          form.hidden = false;
          text.focus();
          return;
        }
        if (action === "delete") {
          var sel = selected();
          var hasScan = sel.some(function (r) { return r.getAttribute("data-source") === "scan"; });
          confirmDelete(sel.length, hasScan).then(function (res) {
            if (res) { runBulk({ action: "delete", ids: ids(), delete_file: res.deleteFile }); }
          });
          return;
        }
        runBulk({ action: action, ids: ids() });
      });
    });
    form.addEventListener("submit", function (ev) {
      ev.preventDefault();
      var value = text.value.trim();
      if (!pending || (pending !== "move" && !value)) { return; }
      var payload = { action: pending, ids: ids() };
      if (pending === "move") { payload.folder = value; } else { payload.tag = value; }
      runBulk(payload);
    });
    $("#bulk-cancel").addEventListener("click", closeAsk);

    // Visibility pill toggles one row.
    $$("button.vis").forEach(function (btn) {
      btn.addEventListener("click", function () {
        var id = btn.closest("tr").getAttribute("data-id");
        var now = btn.getAttribute("data-public") === "1";
        api("PATCH", "/admin/api/media/" + encodeURIComponent(id), { public: !now }).then(reload, fail);
      });
    });
    refresh();
  }

  // Stats row: filled when /admin/api/stats exists.
  function fillStat(id, val) {
    var v = $("#" + id + " .v");
    if (!v || val === undefined || val === null) { return; }
    var main = typeof val === "object" ? val.value : val;
    if (main === undefined || main === null) { return; }
    v.textContent = String(main);
    if (typeof val === "object" && val.detail) {
      var small = document.createElement("small");
      small.textContent = String(val.detail);
      v.appendChild(small);
    }
  }
  if ($("#stat-media")) {
    fetch("/admin/api/stats", { credentials: "same-origin" }).then(function (r) {
      return r.ok ? r.json() : null;
    }).then(function (s) {
      if (!s) { return; }
      fillStat("stat-disk", s.disk);
      fillStat("stat-scan", s.scan);
      fillStat("stat-encoder", s.encoder);
    }, function () {});
  }

  // Processing queue: polled while /admin/api/jobs exists.
  var queue = $("#queue");
  if (queue) {
    var qrows = $("#queue-rows");
    var qempty = $("#queue-empty");
    var timer = null;
    var render = function (jobs) {
      while (qrows.firstChild) { qrows.removeChild(qrows.firstChild); }
      qempty.hidden = jobs.length > 0;
      jobs.forEach(function (j) {
        var row = document.createElement("div");
        row.className = "row";
        var name = document.createElement("span");
        name.className = "n";
        name.textContent = (j.title ? j.title + " · " : "") + (j.kind || "job");
        var track = document.createElement("div");
        track.className = "track";
        var bar = document.createElement("i");
        var p = Number(j.progress);
        bar.style.width = (isFinite(p) ? Math.max(0, Math.min(100, p <= 1 ? p * 100 : p)) : 0) + "%";
        track.appendChild(bar);
        var state = document.createElement("span");
        state.textContent = j.state || "";
        row.appendChild(name);
        row.appendChild(track);
        row.appendChild(state);
        qrows.appendChild(row);
      });
    };
    var poll = function () {
      fetch("/admin/api/jobs", { credentials: "same-origin" }).then(function (r) {
        if (r.status === 404) { clearInterval(timer); return null; }
        return r.ok ? r.json() : null;
      }).then(function (data) {
        if (!data) { return; }
        render(Array.isArray(data) ? data : (data.jobs || data.items || []));
      }, function () {});
    };
    timer = setInterval(poll, 5000);
    poll();
  }

  // ---- Tags page ----
  var tagNew = $("#tag-new");
  if (tagNew) {
    tagNew.addEventListener("submit", function (ev) {
      ev.preventDefault();
      var name = $("#tag-new-name").value.trim();
      if (!name) { return; }
      api("POST", "/admin/api/tags", { name: name, colour: $("#tag-new-colour").value }).then(reload, fail);
    });
  }
  var tagUrl = function (tr) { return "/admin/api/tags/" + encodeURIComponent(tr.getAttribute("data-tag-id")); };
  $$("#tags tr[data-tag-id]").forEach(function (tr) {
    var nameBtn = $(".tag-name", tr);
    nameBtn.addEventListener("click", function () {
      var input = document.createElement("input");
      input.type = "text";
      input.maxLength = 40;
      input.value = tr.getAttribute("data-name");
      input.setAttribute("aria-label", "Tag name");
      var finished = false;
      var commit = function () {
        if (finished) { return; }
        finished = true;
        var v = input.value.trim();
        if (!v || v === tr.getAttribute("data-name")) { input.replaceWith(nameBtn); return; }
        api("PATCH", tagUrl(tr), { name: v }).then(reload, function (e) { fail(e); input.replaceWith(nameBtn); });
      };
      input.addEventListener("keydown", function (ev) {
        if (ev.key === "Enter") { commit(); }
        if (ev.key === "Escape") { finished = true; input.replaceWith(nameBtn); }
      });
      input.addEventListener("blur", commit);
      nameBtn.replaceWith(input);
      input.focus();
      input.select();
    });
    $(".tag-colour", tr).addEventListener("change", function (ev) {
      api("PATCH", tagUrl(tr), { colour: ev.target.value }).then(reload, fail);
    });
    $(".tag-del", tr).addEventListener("click", function () {
      var dlg = $("#tag-dialog");
      $("#tag-dialog-text").textContent = "Delete the tag \"" + tr.getAttribute("data-name") + "\"? It is removed from every item.";
      var ok = $("#tag-dialog-confirm");
      var cancel = $("#tag-dialog-cancel");
      var cleanup = function () {
        ok.removeEventListener("click", onOk);
        cancel.removeEventListener("click", onCancel);
        dlg.removeEventListener("close", cleanup);
      };
      var onOk = function () { cleanup(); dlg.close(); api("DELETE", tagUrl(tr)).then(reload, fail); };
      var onCancel = function () { cleanup(); dlg.close(); };
      ok.addEventListener("click", onOk);
      cancel.addEventListener("click", onCancel);
      dlg.addEventListener("close", cleanup);
      dlg.showModal();
    });
  });

  // ---- Tokens page ----
  var tokNew = $("#token-new");
  if (tokNew) {
    var tokDlg = $("#token-created");
    tokNew.addEventListener("submit", function (ev) {
      ev.preventDefault();
      var name = $("#token-new-name").value.trim();
      if (!name) { return; }
      api("POST", "/admin/api/tokens", { name: name }).then(function (res) {
        $("#token-value").value = res.token;
        tokDlg.addEventListener("close", reload, { once: true });
        tokDlg.showModal();
        $("#token-value").select();
      }, fail);
    });
    $("#token-copy").addEventListener("click", function () {
      var input = $("#token-value");
      input.select();
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(input.value).then(function () { toast("Copied"); }, function () { document.execCommand("copy"); });
      } else {
        document.execCommand("copy");
      }
    });
    $("#token-done").addEventListener("click", function () { tokDlg.close(); });
    $$("#tokens .token-revoke").forEach(function (btn) {
      btn.addEventListener("click", function () {
        var tr = btn.closest("tr");
        var dlg = $("#token-revoke-dialog");
        $("#token-revoke-text").textContent = "Revoke the token \"" + tr.getAttribute("data-name") + "\"? Uploads using it will stop working.";
        var ok = $("#token-revoke-confirm");
        var cancel = $("#token-revoke-cancel");
        var cleanup = function () {
          ok.removeEventListener("click", onOk);
          cancel.removeEventListener("click", onCancel);
          dlg.removeEventListener("close", cleanup);
        };
        var onOk = function () {
          cleanup(); dlg.close();
          api("DELETE", "/admin/api/tokens/" + encodeURIComponent(tr.getAttribute("data-token-id"))).then(reload, fail);
        };
        var onCancel = function () { cleanup(); dlg.close(); };
        ok.addEventListener("click", onOk);
        cancel.addEventListener("click", onCancel);
        dlg.addEventListener("close", cleanup);
        dlg.showModal();
      });
    });
  }

  // ---- Clip page editor ----
  var panel = $(".panel[data-id]");
  if (panel && $("#details-form")) {
    var mediaUrl = "/admin/api/media/" + encodeURIComponent(panel.getAttribute("data-id"));
    $$("[data-visibility]", panel).forEach(function (btn) {
      btn.addEventListener("click", function () {
        if (btn.classList.contains("on")) { return; }
        api("PATCH", mediaUrl, { public: btn.getAttribute("data-visibility") === "public" }).then(reload, fail);
      });
    });
    $("#details-form").addEventListener("submit", function (ev) {
      ev.preventDefault();
      var tagsEl = $("#ed-tags");
      var body = { title: $("#ed-title").value, description: $("#ed-desc").value };
      if (tagsEl.value !== tagsEl.defaultValue) {
        body.tags = tagsEl.value.split(",").map(function (t) { return t.trim(); }).filter(Boolean);
      }
      api("PATCH", mediaUrl, body).then(reload, fail);
    });
    var del = $("[data-delete]", panel);
    if (del) {
      del.addEventListener("click", function () {
        var hasScan = panel.getAttribute("data-source") === "scan";
        confirmDelete(1, hasScan).then(function (res) {
          if (!res) { return; }
          api("DELETE", mediaUrl + "?delete_file=" + (res.deleteFile ? "true" : "false")).then(function () {
            window.location.href = "/";
          }, fail);
        });
      });
    }
  }
})();
