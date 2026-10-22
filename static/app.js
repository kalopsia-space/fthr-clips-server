(function () {
  "use strict";
  var PLACEHOLDER = "/static/poster-placeholder.svg";
  var $$ = function (sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); };

  function flash(el, text) {
    var original = el.getAttribute("data-label") || el.textContent;
    el.setAttribute("data-label", original);
    el.textContent = text;
    el.classList.add("done");
    clearTimeout(el._flashTimer);
    el._flashTimer = setTimeout(function () {
      el.textContent = original;
      el.classList.remove("done");
    }, 1400);
  }

  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.setAttribute("readonly", "");
      ta.className = "sr";
      document.body.appendChild(ta);
      ta.select();
      var ok = false;
      try { ok = document.execCommand("copy"); } catch (e) { ok = false; }
      document.body.removeChild(ta);
      ok ? resolve() : reject(new Error("copy failed"));
    });
  }

  function fmtTime(s) {
    s = Math.max(0, Math.floor(s));
    var m = Math.floor(s / 60);
    return m + ":" + ("0" + (s % 60)).slice(-2);
  }

  // Poster fallback.
  $$("img.poster").forEach(function (img) {
    function swap() {
      if (img.getAttribute("src") !== PLACEHOLDER) { img.setAttribute("src", PLACEHOLDER); }
    }
    img.addEventListener("error", swap);
    if (img.complete && img.naturalWidth === 0) { swap(); }
  });

  // Tag colours arrive as data attributes (inline styles are blocked by CSP).
  $$("[data-colour]").forEach(function (el) {
    var c = el.getAttribute("data-colour");
    if (/^#[0-9a-fA-F]{3,8}$/.test(c)) { el.style.setProperty("--tag", c); }
  });

  // Copy-to-clipboard buttons.
  document.addEventListener("click", function (ev) {
    var el = ev.target.closest ? ev.target.closest("[data-copy]") : null;
    if (!el) { return; }
    ev.preventDefault();
    copyText(el.getAttribute("data-copy")).then(function () {
      if (el.classList.contains("share")) {
        el.classList.add("done");
        setTimeout(function () { el.classList.remove("done"); }, 1400);
      } else {
        flash(el, "Copied");
      }
    }, function () {});
  });

  // Sort select submits its form.
  $$("select[data-autosubmit]").forEach(function (sel) {
    sel.addEventListener("change", function () { if (sel.form) { sel.form.submit(); } });
  });

  // Hover preview on cards.
  $$(".thumb[data-preview]").forEach(function (thumb) {
    var video = null;
    var card = thumb.closest(".card") || thumb;
    card.addEventListener("mouseenter", function () {
      if (!video) {
        video = document.createElement("video");
        video.muted = true;
        video.loop = true;
        video.playsInline = true;
        video.preload = "none";
        video.src = thumb.getAttribute("data-preview");
        var img = thumb.querySelector("img");
        if (img) { img.insertAdjacentElement("afterend", video); } else { thumb.appendChild(video); }
      }
      var p = video.play();
      if (p && p.then) {
        p.then(function () { video.classList.add("on"); }, function () {});
      }
    });
    card.addEventListener("mouseleave", function () {
      if (video) { video.pause(); video.classList.remove("on"); video.currentTime = 0; }
    });
  });

  // Clip page player.
  var player = document.getElementById("player");
  if (player) {
    var start = parseFloat(player.getAttribute("data-start") || "0");
    var seeked = false;
    function seekStart() {
      if (seeked || !(start > 0)) { return; }
      seeked = true;
      if (player.duration && start < player.duration) { player.currentTime = start; }
    }
    if (player.readyState >= 1) { seekStart(); }
    player.addEventListener("loadedmetadata", seekStart);

    var q = document.querySelector("select[data-quality]");
    if (q) {
      q.addEventListener("change", function () {
        var t = player.currentTime;
        var wasPaused = player.paused;
        var opt = q.options[q.selectedIndex];
        var restore = function () {
          player.removeEventListener("loadedmetadata", restore);
          player.currentTime = t;
          if (!wasPaused) { var p = player.play(); if (p && p.catch) { p.catch(function () {}); } }
        };
        player.addEventListener("loadedmetadata", restore);
        player.setAttribute("src", opt.value);
        player.load();
      });
    }

    var atTime = document.querySelector("[data-copy-time]");
    if (atTime) {
      var update = function () { atTime.textContent = "Copy at current time (" + fmtTime(player.currentTime) + ")"; atTime.removeAttribute("data-label"); };
      player.addEventListener("timeupdate", update);
      update();
      atTime.addEventListener("click", function () {
        var url = atTime.getAttribute("data-copy-time") + "?t=" + Math.floor(player.currentTime);
        copyText(url).then(function () { flash(atTime, "Copied"); }, function () {});
      });
    }
  }

  // Image lightbox: click the picture to see it full screen; Esc or click closes.
  var box = document.querySelector(".lightbox");
  var shot = document.querySelector("img[data-lightbox]");
  if (box && shot) {
    var boxImg = box.querySelector("img");
    var closeBox = function () { box.hidden = true; };
    shot.addEventListener("click", function () {
      boxImg.src = shot.getAttribute("data-lightbox");
      box.hidden = false;
    });
    box.addEventListener("click", closeBox);
    document.addEventListener("keydown", function (ev) {
      if (ev.key === "Escape" && !box.hidden) { closeBox(); }
    });
  }
})();
