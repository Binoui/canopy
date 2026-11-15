(() => {
  const hint = field => (field.getAttribute('autocomplete') || '').toLowerCase().split(/\s+/);

  function usable(field) {
    if (field.disabled || field.readOnly || !field.getClientRects().length) return false;
    // A field inside a web component is hidden by whatever hides the component.
    for (let node = field; node; node = node.parentElement || node.getRootNode().host) {
      if (node.hidden || node.inert) return false;
      const style = getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || Number(style.opacity) === 0) return false;
    }
    return true;
  }

  // Every usable input in page order, including those inside open web components, where
  // sites such as Reddit keep their sign-in fields.
  function* elements(root) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      yield node;
      if (node.shadowRoot) yield* elements(node.shadowRoot);
    }
  }
  function inputs() {
    const found = [];
    for (const node of elements(document)) if (node.localName === 'input' && usable(node)) found.push(node);
    return found;
  }
  const isText = field => ['text', 'email', 'tel'].includes(field.type);

  // The field a saved password belongs in. A form with several unmarked password fields
  // creates or changes a password, and is left alone.
  function passwordField(all) {
    const fields = all.filter(field => field.type === 'password' && !hint(field).includes('new-password'));
    const marked = fields.find(field => hint(field).includes('current-password'));
    if (marked) return marked;
    const first = fields[0];
    if (!first) return null;
    return fields.filter(field => field.form === first.form).length === 1 ? first : null;
  }

  // The account name that goes with `password`: the nearest text field before it. With no
  // password on the page, this is the first step of a two-step sign-in: the field the site
  // marks as the user name or as taking a passkey, or the only email or user-name field
  // of a sign-in address.
  function usernameField(all, password) {
    const fields = all.filter(isText);
    if (password) {
      const at = all.indexOf(password);
      const before = fields.filter(field => all.indexOf(field) < at && field.form === password.form);
      return before.findLast(field => hint(field).some(word => word === 'username' || word === 'email')) || before[before.length - 1] || null;
    }
    const marked = fields.find(field => hint(field).some(word => word === 'username' || word === 'webauthn'));
    if (marked) return marked;
    if (!/(log|sign)[-_]?(in|on)|auth|sso|account/i.test(location.hostname + location.pathname + location.hash)) return null;
    const named = fields.filter(field => field.type === 'email' || /user|login|e-?mail|ident/i.test(`${field.name} ${field.id}`));
    return named.length === 1 ? named[0] : null;
  }

  function targets() {
    const all = inputs();
    const password = passwordField(all);
    return { password, username: usernameField(all, password), asks: all.some(field => field.type === 'password') };
  }

  // `asks` is true while the page shows any password field, fillable or not: a page that
  // still asks for a password has not accepted the one just sent.
  function state() {
    const { username, password, asks } = targets();
    return { url: location.href, username: !!username, password: !!password, asks };
  }

  // Sets the field the way typing would, so the page's own scripts notice the value.
  function put(field, value) {
    field.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(field, value);
    field.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    field.dispatchEvent(new Event('change', { bubbles: true }));
  }

  // Fills only a document of the origin the login was saved for; the check and the fill
  // happen in the same turn, so a navigation cannot slip between them.
  function fill(origin, username, password) {
    if (location.origin !== origin) return 'origin';
    const { username: usernameTarget, password: passwordTarget } = targets();
    if (!passwordTarget && !usernameTarget) return 'none';
    if (usernameTarget && username) put(usernameTarget, username);
    if (passwordTarget) put(passwordTarget, password);
    return passwordTarget ? 'filled' : 'username';
  }

  // Tells Canopy when sign-in fields appear or go away. The message carries nothing:
  // Canopy asks for the state itself, against the document it knows.
  let reported = 'false/false/false';
  let timer = 0;
  function check() {
    const now = state();
    const key = `${now.username}/${now.password}/${now.asks}`;
    if (key === reported) return;
    reported = key;
    try { webkit.messageHandlers.canopyLoginFields.postMessage(''); } catch (_) {}
  }
  // At most one look every 300 ms, however busy the page is.
  function soon() {
    timer ||= setTimeout(() => {
      timer = 0;
      check();
    }, 300);
  }
  new MutationObserver(soon).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['class', 'style', 'hidden', 'type', 'disabled', 'readonly'] });
  addEventListener('DOMContentLoaded', soon);
  addEventListener('load', soon);
  addEventListener('focusin', soon, true);
  // A page returning from the page cache has a new owner on Canopy's side.
  addEventListener('pageshow', event => {
    if (event.persisted) {
      reported = 'false/false/false';
      soon();
    }
  });

  // ---------- a login the user sends ----------

  // The password being sent: the only one typed, the one marked as new, or the one typed
  // twice on a form that asks for it again.
  function sentPassword(all) {
    const typed = all.filter(field => field.type === 'password' && field.value);
    if (typed.length < 2) return typed[0] || null;
    return typed.find(field => hint(field).includes('new-password'))
      || typed.find((field, index) => typed.some((other, at) => at > index && other.value === field.value))
      || typed[0];
  }

  // The second step of a two-step sign-in keeps the user name in a field it does not show.
  function keptUsername(password) {
    for (const field of (password.form || document).querySelectorAll('input')) {
      if (field.type === 'password' || !field.value) continue;
      if (hint(field).includes('username') || (field.type === 'hidden' && /^(user(name)?|login(fmt)?|e-?mail|identifier)$/i.test(field.name))) return field.value;
    }
    return '';
  }

  // Tells Canopy the user name and password as the user sends them, since the page may be
  // gone a moment later. Canopy offers to save them only once the page stops asking.
  let sent = '';
  function send() {
    const all = inputs();
    const password = sentPassword(all);
    if (!password) return;
    const username = usernameField(all, password)?.value || keptUsername(password);
    const message = JSON.stringify({ origin: location.origin, username, password: password.value });
    if (message === sent) return;
    sent = message;
    try { webkit.messageHandlers.canopyLoginSent.postMessage(message); } catch (_) {}
  }
  const source = event => event.composedPath()[0];
  addEventListener('submit', send, true);
  addEventListener('click', event => {
    if (event.isTrusted && source(event)?.closest?.('button,input[type="submit"],input[type="button"],[role="button"],a')) send();
  }, true);
  addEventListener('keydown', event => {
    if (event.isTrusted && event.key === 'Enter' && source(event)?.localName === 'input') send();
  }, true);

  Object.defineProperty(globalThis, '__canopyLogins', { value: Object.freeze({ state, fill }) });
})();
