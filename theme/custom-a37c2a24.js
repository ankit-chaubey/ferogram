/* ferogram docs  Custom JS */

// ── Font size default ────────────────────────────────────────
if (!localStorage.getItem('mdbook-font-size')) {
  localStorage.setItem('mdbook-font-size', '2');
}

document.addEventListener('DOMContentLoaded', function () {

  // ── Callout boxes ──────────────────────────────────────────
  // Usage in markdown:  > **TIP** your text here
  //                     > **SECURITY:** your text here
  const callouts = {
    'NOTE':     { cls: 'callout-note',    icon: '📝', label: 'Note' },
    'INFO':     { cls: 'callout-info',    icon: 'ℹ️',  label: 'Info' },
    'TIP':      { cls: 'callout-tip',     icon: '💡', label: 'Tip' },
    'WARNING':  { cls: 'callout-warning', icon: '⚠️', label: 'Warning' },
    'DANGER':   { cls: 'callout-danger',  icon: '🚨', label: 'Danger' },
    'SECURITY': { cls: 'callout-danger',  icon: '🔒', label: 'Security' },
  };

  document.querySelectorAll('blockquote').forEach(bq => {
    const firstP = bq.querySelector('p:first-child');
    if (!firstP) return;

    // The keyword lives inside a <strong> as the very first child
    const strongEl = firstP.querySelector('strong:first-child');
    if (!strongEl) return;

    const keyword = strongEl.textContent.trim().replace(/:$/, '').toUpperCase();
    const meta = callouts[keyword];
    if (!meta) return;

    // ── Remove the <strong> keyword element so it doesn't duplicate ──
    strongEl.remove();

    // Also strip any leading colon + whitespace left in the text node after strong
    firstP.childNodes.forEach(node => {
      if (node.nodeType === Node.TEXT_NODE) {
        node.textContent = node.textContent.replace(/^:\s*/, '').trimStart();
      }
    });

    // ── Build the callout card ───────────────────────────────
    const wrapper = document.createElement('div');
    wrapper.className = 'callout ' + meta.cls;

    const iconEl = document.createElement('div');
    iconEl.className = 'callout-icon';
    iconEl.textContent = meta.icon;

    const contentEl = document.createElement('div');
    contentEl.className = 'callout-content';

    const labelEl = document.createElement('strong');
    labelEl.textContent = meta.label;
    contentEl.appendChild(labelEl);

    // Move all blockquote children into contentEl
    while (bq.firstChild) contentEl.appendChild(bq.firstChild);

    wrapper.appendChild(iconEl);
    wrapper.appendChild(contentEl);
    bq.parentNode.replaceChild(wrapper, bq);
  });

  // ── Language label on code blocks ─────────────────────────
  document.querySelectorAll('pre code[class*="language-"]').forEach(code => {
    const lang = (code.className.match(/language-(\w+)/) || [])[1];
    if (!lang || lang === 'text') return;
    const label = document.createElement('span');
    label.style.cssText = `
      position:absolute; top:10px; right:42px;
      font-size:11px; font-weight:600; letter-spacing:0.08em;
      text-transform:uppercase; color:rgba(255,255,255,0.22);
      font-family:'JetBrains Mono',monospace; pointer-events:none;
    `;
    label.textContent = lang;
    code.parentElement.style.position = 'relative';
    code.parentElement.appendChild(label);
  });

  // ── Smooth scroll for anchor links ────────────────────────
  document.querySelectorAll('a[href^="#"]').forEach(a => {
    a.addEventListener('click', e => {
      const target = document.querySelector(a.getAttribute('href'));
      if (target) {
        e.preventDefault();
        target.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    });
  });

});
