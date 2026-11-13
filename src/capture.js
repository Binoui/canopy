(() => {
  const limit = (value, size) => Array.from(String(value || '')).slice(0, size).join('');
  const compact = value => limit(String(value || '').replace(/\s+/g, ' ').trim(), 1000);
  const excluded = 'input,textarea,select,[contenteditable]:not([contenteditable="false"])';
  const postSelector = 'shreddit-comment,.comment,[data-comment-id],[data-testid="tweet"],article,[role="article"]';
  const contexts = [];
  let lastPointer = null;

  function element(node) {
    return node && (node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement);
  }

  function cleanText(node) {
    if (node.nodeType === Node.ELEMENT_NODE && node.closest(excluded)) return '';
    const copy = node.cloneNode(true);
    if (copy.querySelectorAll) {
      copy.querySelectorAll(excluded + ',script,style,noscript,nav,header,footer,[hidden],[aria-hidden="true"]').forEach(item => item.remove());
      copy.querySelectorAll('a').forEach(item => item.append(' '));
      copy.querySelectorAll('p,div,li,blockquote,pre,h1,h2,h3,h4,section,article').forEach(item => item.append('\n'));
    }
    return String(copy.textContent || '').replace(/[\t ]+/g, ' ').replace(/\n[\t ]*/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
  }

  function citation(container) {
    if (!container) return null;
    const candidates = [];
    for (const name of ['data-permalink', 'data-comment-permalink', 'permalink']) {
      if (container.hasAttribute(name)) candidates.push(container.getAttribute(name));
    }
    for (const candidate of container.querySelectorAll('a[rel="bookmark"],a.permalink,a[data-testid="comment_permalink"],a[slot="comment-time"]')) candidates.push(candidate.href);
    for (const stamp of container.querySelectorAll('time,faceplate-timeago')) {
      const anchor = stamp.closest('a[href]');
      if (anchor && container.contains(anchor)) candidates.push(anchor.href);
    }
    for (const candidate of candidates) {
      try {
        const url = new URL(candidate, location.href);
        if (url.origin === location.origin && /^https?:$/.test(url.protocol)) return url.href;
      } catch (_) {}
    }
    if (container.id) {
      const url = new URL(location.href);
      url.hash = container.id;
      return url.href;
    }
    return null;
  }

  function record(event) {
    if (!event.isTrusted || (event.button !== 0 && event.button !== 1)) return;
    const target = element(event.composedPath ? event.composedPath()[0] : event.target);
    if (target?.closest(excluded)) return;
    const anchor = target && target.closest('a[href]');
    if (!anchor || anchor.hasAttribute('download')) return;
    let url;
    try { url = new URL(anchor.href, location.href); } catch (_) { return; }
    if (!/^https?:$/.test(url.protocol)) return;
    const now = Date.now();
    if (event.type !== 'mousedown' && lastPointer && lastPointer.anchor === anchor && now - lastPointer.at < 1000) return;
    if (event.type === 'mousedown') lastPointer = { anchor, at: now };
    const container = anchor.closest(postSelector) || anchor.closest('p,li,blockquote') || anchor.parentElement;
    contexts.push({ url: url.href, page_url: location.href, link_text: limit(anchor.textContent.trim(), 300),
      excerpt: compact(container ? cleanText(container) : ''), citation_url: citation(container), at: now });
    while (contexts.length > 12) contexts.shift();
  }

  document.addEventListener('mousedown', record, true);
  document.addEventListener('click', record, true);
  document.addEventListener('auxclick', record, true);

  function takeContext(url) {
    const now = Date.now();
    while (contexts.length && now - contexts[0].at > 10000) contexts.shift();
    const matching = contexts.filter(item => item.url === url && item.page_url === location.href);
    if (!matching.length) return null;
    const first = matching[0];
    const ambiguous = matching.some(item => item.excerpt !== first.excerpt || item.citation_url !== first.citation_url);
    if (ambiguous) {
      for (let index = contexts.length - 1; index >= 0; index--) if (contexts[index].url === url) contexts.splice(index, 1);
      return null;
    }
    contexts.splice(contexts.indexOf(first), 1);
    return first;
  }

  function capture(selectionOnly) {
    const result = { document_url: location.href, title: limit(document.title, 1000), text: '', citation_url: null, blocked: false };
    const visiblePassword = Array.from(document.querySelectorAll('input[type="password"]')).some(field => {
      if (field.closest('[hidden],[inert]') || !field.getClientRects().length) return false;
      for (let node = field; node; node = node.parentElement) {
        const style = getComputedStyle(node);
        if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || Number(style.opacity) === 0) return false;
      }
      return true;
    });
    if (visiblePassword) {
      result.blocked = true;
      return result;
    }
    if (selectionOnly) {
      const selection = getSelection();
      if (!selection || selection.isCollapsed || !selection.rangeCount) return result;
      const pieces = [];
      for (let index = 0; index < selection.rangeCount; index++) {
        const range = selection.getRangeAt(index);
        if ([range.startContainer, range.endContainer, range.commonAncestorContainer].some(node => element(node)?.closest(excluded))) {
          result.blocked = true;
          return result;
        }
        pieces.push(cleanText(range.cloneContents()));
      }
      result.text = limit(pieces.join('\n').trim(), 200000);
      result.citation_url = citation(element(selection.getRangeAt(0).commonAncestorContainer)?.closest(postSelector));
    } else {
      const root = document.querySelector('main,[role="main"]') || document.querySelector('article') || document.body;
      if (root) result.text = limit(cleanText(root), 200000);
    }
    return result;
  }

  Object.defineProperty(globalThis, '__canopyMemory', { value: Object.freeze({ takeContext, capture }) });
})();
