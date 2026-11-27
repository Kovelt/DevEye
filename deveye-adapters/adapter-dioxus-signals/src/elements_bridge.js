// Pont d'éléments DevEye (ADR-0074) : désigner un élément du DOM par une clé
// stable, sans CDP. Évalué en SYNCHRONE par la pompe `document::eval` du SDK
// (pas d'awaitPromise) : aucune Promise, aucune attente.
//
// Le script est une EXPRESSION (IIFE sans point-virgule final) : il s'installe
// dans `window.__deveye_elements__` et renvoie l'API. Il n'a aucune dépendance
// et reste lisible par un navigateur sans CDP (WebKitGTK 2.34+).
//
// Plus fruste que l'arbre d'accessibilité du navigateur : rôle implicite par une
// table minimale, nom accessible simplifié, pas de traversée des iframes.
(function () {
  'use strict';
  var VERSION = 1;
  var existing = window.__deveye_elements__;
  if (existing && existing.version === VERSION) return existing;

  var NAME_MAX = 120;
  var VALUE_MAX = 200;
  var VISIT_CAP = 50000;
  var MAX_DEPTH = 300;
  // Profondeur de SORTIE (en nœuds) : chaque niveau coûte deux niveaux JSON
  // (objet puis tableau `children`) et le décodeur serde_json côté serveur refuse
  // au-delà de 128 ; 50 niveaux laissent de la marge pour l'enveloppe et `state`.
  var MAX_OUTPUT_DEPTH = 50;

  // Une clé = `<epoch>:<n>`. L'epoch change à chaque chargement de page : une
  // clé d'une page précédente est reconnue comme étrangère, jamais réattribuée.
  var epoch = Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
  var counter = 0;
  var ids = new WeakMap(); // élément -> n
  var live = new Map(); // n -> WeakRef(élément)
  var weak = typeof WeakRef === 'function'
    ? function (el) { return new WeakRef(el); }
    : function (el) { return { deref: function () { return el; } }; };
  var registry = typeof FinalizationRegistry === 'function'
    ? new FinalizationRegistry(function (n) { live.delete(n); })
    : null;

  function keyOf(el) {
    var n = ids.get(el);
    if (n === undefined) {
      n = ++counter;
      ids.set(el, n);
      live.set(n, weak(el));
      if (registry) registry.register(el, n);
    }
    return epoch + ':' + n;
  }

  // { el } si vivant et rattaché, { foreign } si la clé vient d'une autre page,
  // { gone } sinon.
  function resolve(key) {
    var i = typeof key === 'string' ? key.indexOf(':') : -1;
    if (i < 0) return { gone: true };
    if (key.slice(0, i) !== epoch) return { foreign: true };
    var ref = live.get(Number(key.slice(i + 1)));
    var el = ref ? ref.deref() : undefined;
    if (!el || !el.isConnected) return { gone: true };
    return { el: el };
  }

  function squash(s) { return (s == null ? '' : String(s)).replace(/\s+/g, ' ').trim(); }
  function clip(s, max) {
    s = squash(s);
    if (s.length <= max) return s;
    var cut = s.slice(0, max - 1);
    // Une surrogate haute finale (paire coupée en deux) rendrait le JSON
    // illisible pour serde_json : on la retire.
    var last = cut.charCodeAt(cut.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) cut = cut.slice(0, -1);
    return cut + '…';
  }

  var INPUT_ROLES = {
    button: 'button', submit: 'button', reset: 'button', image: 'button',
    checkbox: 'checkbox', radio: 'radio', range: 'slider', number: 'spinbutton',
    search: 'searchbox', email: 'textbox', tel: 'textbox', url: 'textbox',
    text: 'textbox', password: 'textbox'
  };
  var TAG_ROLES = {
    button: 'button', summary: 'button', textarea: 'textbox', option: 'option',
    h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading',
    nav: 'navigation', main: 'main', aside: 'complementary', header: 'banner',
    footer: 'contentinfo', form: 'form', dialog: 'dialog', ul: 'list', ol: 'list',
    li: 'listitem', table: 'table', tr: 'row', td: 'cell', th: 'columnheader',
    progress: 'progressbar', meter: 'meter', details: 'group', fieldset: 'group',
    hr: 'separator'
  };
  var INTERACTIVE = {
    button: 1, link: 1, checkbox: 1, radio: 1, textbox: 1, searchbox: 1, spinbutton: 1,
    slider: 1, combobox: 1, listbox: 1, option: 1, tab: 1, menuitem: 1,
    menuitemcheckbox: 1, menuitemradio: 1, switch: 1, treeitem: 1, clickable: 1
  };
  // Gardés en mode `interactive_only`, même sans action : ils situent l'agent.
  var LANDMARK = {
    heading: 1, navigation: 1, main: 1, complementary: 1, banner: 1,
    contentinfo: 1, dialog: 1, form: 1
  };
  // Conteneurs gardés seulement s'ils portent des éléments gardés.
  var CONTEXT = {
    list: 1, menu: 1, menubar: 1, tablist: 1, toolbar: 1, tree: 1, listbox: 1,
    group: 1, table: 1, grid: 1, radiogroup: 1
  };
  // Rôles dont le nom vient du contenu.
  var NAME_FROM_CONTENT = {
    button: 1, link: 1, heading: 1, tab: 1, menuitem: 1, menuitemcheckbox: 1,
    menuitemradio: 1, option: 1, switch: 1, treeitem: 1, columnheader: 1, clickable: 1,
    cell: 1, listitem: 1, checkbox: 1, radio: 1
  };
  var SKIP = {
    script: 1, style: 1, template: 1, noscript: 1, head: 1, meta: 1, link: 1,
    title: 1, base: 1, svg: 1
  };

  function roleOf(el) {
    var explicit = el.getAttribute('role');
    if (explicit) {
      var first = explicit.trim().split(/\s+/)[0].toLowerCase();
      if (first === 'presentation' || first === 'none') return null;
      if (first) return first;
    }
    var tag = el.localName;
    if (tag === 'a' || tag === 'area') return el.hasAttribute('href') ? 'link' : null;
    if (tag === 'input') {
      var t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'hidden') return null;
      return INPUT_ROLES[t] || 'textbox';
    }
    if (tag === 'select') return (el.multiple || el.size > 1) ? 'listbox' : 'combobox';
    if (tag === 'img') return el.getAttribute('alt') === '' ? null : 'img';
    if (TAG_ROLES[tag]) return TAG_ROLES[tag];
    var ce = el.getAttribute('contenteditable');
    if (ce !== null && ce.toLowerCase() !== 'false') return 'textbox';
    return null;
  }

  // Filet heuristique (ADR-0064, ADR-0074) : la page n'a pas de notion de secret,
  // seulement des indices. Les listes reprennent celles du masquage par noms de
  // clés du wire protocol (`docs/schemas/shared-vectors-1.json`) ; un test dans un
  // vrai navigateur vérifie chaque clé requise des vecteurs partagés.
  var SECRET_CONTAINS = [
    'password', 'passwd', 'secret', 'apikey', 'privatekey', 'credential',
    'authorization', 'accesskey', 'signingkey', 'encryptionkey', 'databaseurl',
    'connectionstring', 'motdepasse', 'mdp', 'jeton', 'passphrase', 'passcode',
    'cardnumber', 'creditcard', 'cvv', 'cvc'
  ];
  var SECRET_SUFFIXES = ['token', 'cookie', 'sessionid'];
  var SECRET_EXACT = ['pwd', 'auth', 'jwt', 'bearer', 'otp', 'tokens', 'cookies', 'pin', 'iban'];
  var SECRET_AUTOCOMPLETE =
    /(^|\s)(current-password|new-password|one-time-code|cc-number|cc-csc|cc-exp|cc-exp-month|cc-exp-year)(\s|$)/i;
  // Champs vus un jour en type=password : un bouton « afficher » les bascule en
  // type=text, ils restent secrets.
  var wasPassword = typeof WeakSet === 'function' ? new WeakSet() : null;

  function secretWord(text) {
    var k = String(text || '').toLowerCase().replace(/[^a-z0-9]/g, '');
    if (!k) return false;
    var i;
    for (i = 0; i < SECRET_CONTAINS.length; i++) if (k.indexOf(SECRET_CONTAINS[i]) >= 0) return true;
    for (i = 0; i < SECRET_SUFFIXES.length; i++) {
      var s = SECRET_SUFFIXES[i];
      if (k.length >= s.length && k.lastIndexOf(s) === k.length - s.length) return true;
    }
    return SECRET_EXACT.indexOf(k) >= 0;
  }

  function isSecretField(el) {
    // Opt-out explicite posé par l'app, sur le champ ou un ancêtre.
    var marked = el.closest ? el.closest('[data-deveye-secret]') : null;
    if (marked && (marked.getAttribute('data-deveye-secret') || '').toLowerCase() !== 'false') return true;
    if (el.localName !== 'input' && el.localName !== 'textarea') return false;
    if ((el.getAttribute('type') || '').toLowerCase() === 'password') {
      if (wasPassword) wasPassword.add(el);
      return true;
    }
    if (wasPassword && wasPassword.has(el)) return true;
    if (SECRET_AUTOCOMPLETE.test(el.getAttribute('autocomplete') || '')) return true;
    return secretWord(el.getAttribute('name')) || secretWord(el.id) ||
      secretWord(el.getAttribute('aria-label')) || secretWord(el.getAttribute('placeholder'));
  }

  function contentText(el) {
    var t = el.innerText;
    if (t == null || squash(t) === '') t = el.textContent;
    return squash((t || '').slice(0, NAME_MAX * 4));
  }

  function idText(el, id) {
    var root = el.getRootNode();
    var target = root && root.getElementById ? root.getElementById(id) : document.getElementById(id);
    return target ? contentText(target) : '';
  }

  function nameOf(el, role) {
    var lb = el.getAttribute('aria-labelledby');
    if (lb) {
      var joined = squash(lb.split(/\s+/).map(function (id) { return idText(el, id); }).join(' '));
      if (joined) return clip(joined, NAME_MAX);
    }
    var al = el.getAttribute('aria-label');
    if (squash(al)) return clip(al, NAME_MAX);
    var tag = el.localName;
    if (tag === 'input' || tag === 'textarea' || tag === 'select') {
      var labels = el.labels;
      if (labels && labels.length) {
        var lt = squash(Array.prototype.map.call(labels, function (l) { return l.textContent; }).join(' '));
        if (lt) return clip(lt, NAME_MAX);
      }
      var type = (el.getAttribute('type') || '').toLowerCase();
      if (tag === 'input' && (type === 'button' || type === 'submit' || type === 'reset')) {
        var v = squash(el.value);
        if (v) return clip(v, NAME_MAX);
      }
      if (tag === 'input' && type === 'image' && squash(el.alt)) return clip(el.alt, NAME_MAX);
    } else if (tag === 'img' || tag === 'area') {
      if (squash(el.getAttribute('alt'))) return clip(el.getAttribute('alt'), NAME_MAX);
    } else if (tag === 'fieldset') {
      var legend = el.querySelector('legend');
      if (legend && squash(legend.textContent)) return clip(legend.textContent, NAME_MAX);
    }
    if (NAME_FROM_CONTENT[role]) {
      var c = contentText(el);
      if (c) return clip(c, NAME_MAX);
    }
    if (squash(el.getAttribute('title'))) return clip(el.getAttribute('title'), NAME_MAX);
    if (squash(el.getAttribute('placeholder'))) return clip(el.getAttribute('placeholder'), NAME_MAX);
    return null;
  }

  function valueOf(el, role) {
    // Jamais la valeur d'un champ reconnu comme secret (heuristique, voir plus haut).
    if (isSecretField(el)) return null;
    var tag = el.localName;
    var v = null;
    if (tag === 'input') {
      var t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'checkbox' || t === 'radio' || t === 'button' || t === 'submit' ||
          t === 'reset' || t === 'image' || t === 'file' || t === 'hidden') return null;
      v = el.value;
    } else if (tag === 'textarea') {
      v = el.value;
    } else if (tag === 'select') {
      v = Array.prototype.map.call(el.selectedOptions || [], function (o) { return o.text; }).join(', ');
    } else if (role === 'slider' || role === 'spinbutton' || role === 'progressbar' || role === 'meter') {
      v = el.getAttribute('aria-valuetext') || el.getAttribute('aria-valuenow');
    } else if (role === 'textbox' && el.hasAttribute('contenteditable')) {
      v = el.textContent;
    }
    return v == null || v === '' ? null : clip(v, VALUE_MAX);
  }

  function isDisabled(el) {
    if (el.getAttribute('aria-disabled') === 'true') return true;
    try { return el.matches(':disabled'); } catch (e) { return false; }
  }

  function deepActive() {
    var a = document.activeElement;
    while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement;
    return a;
  }

  function triState(el, role) {
    if (el.localName === 'input') {
      var t = (el.getAttribute('type') || '').toLowerCase();
      if (t === 'checkbox' || t === 'radio') return el.indeterminate ? null : !!el.checked;
    }
    var ac = el.getAttribute('aria-checked');
    if (ac === 'true') return true;
    if (ac === 'false') return false;
    return null;
  }

  function expandedOf(el) {
    var ae = el.getAttribute('aria-expanded');
    if (ae === 'true') return true;
    if (ae === 'false') return false;
    if (el.localName === 'summary' && el.parentElement && el.parentElement.localName === 'details') {
      return !!el.parentElement.open;
    }
    return null;
  }

  function rectOf(el) {
    var r = el.getBoundingClientRect();
    return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
  }

  function isOffscreen(rect) {
    var right = rect[0] + rect[2];
    var bottom = rect[1] + rect[3];
    return right <= 0 || bottom <= 0 || rect[0] >= window.innerWidth || rect[1] >= window.innerHeight;
  }

  // Le centre (point où l'on clique) est hors de la fenêtre, même si le rect la
  // recouvre en partie : une grande carte à moitié visible.
  function centerOut(rect) {
    var cx = rect[0] + Math.floor(rect[2] / 2);
    var cy = rect[1] + Math.floor(rect[3] / 2);
    return cx < 0 || cy < 0 || cx >= window.innerWidth || cy >= window.innerHeight;
  }

  function isScrollable(el, cs) {
    var ov = (cs.overflowY || '') + ' ' + (cs.overflowX || '');
    return /(auto|scroll)/.test(ov) &&
      (el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1);
  }

  function childrenOf(el) {
    if (el.shadowRoot) return Array.prototype.slice.call(el.shadowRoot.children);
    if (el.localName === 'slot' && typeof el.assignedElements === 'function') {
      var assigned = el.assignedElements({ flatten: true });
      if (assigned.length) return assigned;
    }
    return Array.prototype.slice.call(el.children);
  }

  function parentCursorIsPointer(el) {
    var p = el.parentElement;
    return p ? getComputedStyle(p).cursor === 'pointer' : false;
  }

  function visit(el, ctx, level) {
    if (ctx.visited++ > VISIT_CAP || level > MAX_DEPTH) { ctx.truncated = true; return []; }
    if (SKIP[el.localName]) return [];
    var cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.contentVisibility === 'hidden') return [];
    if (el.getAttribute('aria-hidden') === 'true') return [];
    var parent = el.parentElement;
    if (parent && parent.localName === 'details' && !parent.open && el.localName !== 'summary') return [];

    var nested = [];
    var kids = childrenOf(el);
    for (var i = 0; i < kids.length; i++) {
      var out = visit(kids[i], ctx, level + 1);
      for (var j = 0; j < out.length; j++) nested.push(out[j]);
    }

    var role = roleOf(el);
    if (!role) {
      // Zone cliquable sans rôle : curseur « main » posé ici et pas hérité du parent.
      var tabindex = el.getAttribute('tabindex');
      // Seul un tabindex strictement positif compte (tabindex="0" et "" désignent
      // un conteneur simplement focusable, pas une zone d'action).
      if ((tabindex !== null && tabindex.trim() !== '' && Number(tabindex) > 0) ||
          (cs.cursor === 'pointer' && !parentCursorIsPointer(el))) {
        role = 'clickable';
      }
    }
    if (!role) return nested;

    var interactive = !!INTERACTIVE[role];
    var keep;
    if (!ctx.interactiveOnly) keep = true;
    else keep = interactive || !!LANDMARK[role] || (!!CONTEXT[role] && nested.length > 0);
    if (!keep) return nested;

    var rect = rectOf(el);
    // Rien de dessiné : l'élément ne se désigne pas (ses enfants, eux, remontent).
    if (cs.visibility === 'hidden' || (rect[2] === 0 && rect[3] === 0 && cs.display !== 'contents')) {
      return nested;
    }
    var offscreen = isOffscreen(rect);
    if (ctx.viewportOnly && offscreen) return nested;

    var key = keyOf(el);
    var name = nameOf(el, role);
    // Un titre ou un point de repère sans nom n'apprend rien à l'agent.
    if (ctx.interactiveOnly && !interactive && !name && nested.length === 0) return nested;

    var focused = el === ctx.active;
    if (focused) ctx.focus = key;
    var node = {
      key: key,
      role: role,
      name: name,
      value: valueOf(el, role),
      state: {
        disabled: isDisabled(el),
        checked: triState(el, role),
        expanded: expandedOf(el),
        focused: focused,
        selected: el.getAttribute('aria-selected') === 'true' || (el.localName === 'option' && !!el.selected)
      },
      rect: rect,
      scrollable: isScrollable(el, cs),
      offscreen: offscreen,
      children: nested
    };
    return [node];
  }

  // Coupe à `depth` niveaux ; vrai si quelque chose a été retranché.
  function limitDepth(nodes, remaining) {
    if (remaining === 0) {
      var cut = nodes.length > 0;
      nodes.length = 0;
      return cut;
    }
    var any = false;
    for (var i = 0; i < nodes.length; i++) any = limitDepth(nodes[i].children, remaining - 1) || any;
    return any;
  }

  // Garde les `budget` premiers nœuds dans l'ordre du document ; renvoie le reste.
  function limitNodes(nodes, budget) {
    var i = 0;
    for (; i < nodes.length; i++) {
      if (budget <= 0) break;
      budget -= 1;
      budget = limitNodes(nodes[i].children, budget);
    }
    if (i < nodes.length) nodes.length = i;
    return budget;
  }

  function count(nodes) {
    var n = 0;
    for (var i = 0; i < nodes.length; i++) n += 1 + count(nodes[i].children);
    return n;
  }

  function snapshot(q) {
    q = q || {};
    var root = document.documentElement;
    if (q.scope_key) {
      var r = resolve(q.scope_key);
      if (r.foreign) return { epoch: epoch, error: 'foreign' };
      if (r.gone) return { epoch: epoch, error: 'gone' };
      root = r.el;
    } else if (q.scope_selector) {
      try { root = document.querySelector(q.scope_selector); } catch (e) {
        return { epoch: epoch, error: 'bad_selector' };
      }
      if (!root) return { epoch: epoch, error: 'scope_not_found' };
    }
    var ctx = {
      interactiveOnly: q.interactive_only !== false,
      viewportOnly: !!q.viewport_only,
      active: deepActive(),
      focus: null,
      visited: 0,
      truncated: false
    };
    var nodes = visit(root, ctx, 0);
    // 0 est traité comme 1 (même règle que les widgets natifs) ; la sortie est
    // bornée à MAX_OUTPUT_DEPTH niveaux quoi qu'on demande.
    var depth = MAX_OUTPUT_DEPTH;
    if (typeof q.max_depth === 'number' && q.max_depth >= 0) {
      depth = Math.min(Math.max(1, Math.floor(q.max_depth)), MAX_OUTPUT_DEPTH);
    }
    if (limitDepth(nodes, depth)) ctx.truncated = true;
    var budget = typeof q.max_nodes === 'number' ? q.max_nodes : 400;
    var total = count(nodes);
    if (total > budget) {
      limitNodes(nodes, budget);
      ctx.truncated = true;
    }
    return { epoch: epoch, nodes: nodes, truncated: ctx.truncated, focus: ctx.focus };
  }

  function inside(outer, inner) {
    while (inner) {
      if (inner === outer) return true;
      inner = inner.parentNode || inner.host;
    }
    return false;
  }

  function topAt(x, y) {
    var top = document.elementFromPoint(x, y);
    while (top && top.shadowRoot) {
      var deeper = top.shadowRoot.elementFromPoint(x, y);
      if (!deeper || deeper === top) break;
      top = deeper;
    }
    return top;
  }

  function covered(el, rect) {
    var cx = rect[0] + Math.floor(rect[2] / 2);
    var cy = rect[1] + Math.floor(rect[3] / 2);
    var top = topAt(cx, cy);
    if (!top) return null;
    if (inside(el, top) || inside(top, el)) return null;
    return top;
  }

  function inScroller(el) {
    for (var p = el.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      var o = getComputedStyle(p);
      var scrolls = /(auto|scroll)/.test(o.overflowY + o.overflowX);
      if (scrolls && (p.scrollHeight > p.clientHeight || p.scrollWidth > p.clientWidth)) return true;
    }
    return false;
  }

  function locate(key) {
    var r = resolve(key);
    if (r.foreign) return { epoch: epoch, state: 'foreign' };
    if (r.gone) return { epoch: epoch, state: 'gone' };
    var el = r.el;
    var rect = rectOf(el);
    var hidden = rect[2] === 0 && rect[3] === 0;
    // Hors vue ou masqué par un conteneur qui défile : on amène l'élément au
    // centre (sans animation, pour mesurer tout de suite), comme un humain.
    // Un simple recouvrement ne fait défiler que si l'élément vit dans un
    // conteneur qui défile (sinon on déplacerait la page sous une modale pour rien).
    if (isOffscreen(rect) || (!hidden && (centerOut(rect) || (inScroller(el) && covered(el, rect))))) {
      try { el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); }
      catch (e) { el.scrollIntoView(); }
      rect = rectOf(el);
    }
    var offscreen = isOffscreen(rect) || centerOut(rect);
    var out = { epoch: epoch, state: 'ok', rect: rect, offscreen: offscreen, disabled: isDisabled(el) };
    if (!offscreen) {
      var top = covered(el, rect);
      if (top) { out.state = 'obscured'; out.by = keyOf(top); }
    }
    return out;
  }

  var api = { version: VERSION, epoch: epoch, snapshot: snapshot, locate: locate };
  window.__deveye_elements__ = api;
  return api;
})()
