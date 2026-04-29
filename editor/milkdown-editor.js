import { Crepe } from '@milkdown/crepe';
import { getHTML } from '@milkdown/kit/utils';


function resolveUploadUri(url, appSlug) {
  if (url && url.startsWith('upload:')) {
    return '/apps/' + appSlug + '/uploads/file/' + url.slice(7);
  }
  return url;
}

async function uploadFile(file, appSlug) {
  var form = new FormData();
  form.append('file', file);
  var resp = await fetch('/apps/' + appSlug + '/uploads', {
    method: 'POST',
    body: form,
  });
  if (!resp.ok) {
    throw new Error('Upload failed: ' + resp.status);
  }
  var data = await resp.json();
  return 'upload:' + data.hash + '/' + data.filename;
}

var activeCrepe = null;

function openEditor(field) {
  var name = field.dataset.richtextName;
  var appSlug = field.dataset.richtextApp;
  var hiddenInput = field.querySelector('input[type="hidden"]');
  var preview = field.querySelector('[data-richtext-preview]');
  var overlay = document.getElementById('richtext-overlay-' + name);
  var modal = document.getElementById('richtext-modal-' + name);
  var root = modal.querySelector('[data-richtext-root]');

  var currentValue = hiddenInput.value ? JSON.parse(hiddenInput.value) : null;
  var markdown = currentValue ? currentValue.markdown : '';

  overlay.classList.add('is-open');
  modal.classList.add('is-open');
  document.body.style.overflow = 'hidden';

  var crepe = new Crepe({
    root: root,
    defaultValue: markdown,
    featureConfigs: {
      [Crepe.Feature.ImageBlock]: {
        onUpload: function(file) {
          return uploadFile(file, appSlug);
        },
        proxyDomURL: function(url) {
          return resolveUploadUri(url, appSlug);
        },
      },
    },
  });

  activeCrepe = crepe;

  crepe.create();

  function close() {
    overlay.classList.remove('is-open');
    modal.classList.remove('is-open');
    document.body.style.overflow = '';
    if (activeCrepe) {
      activeCrepe.destroy();
      activeCrepe = null;
    }
    root.replaceChildren();
  }

  modal.querySelector('[data-richtext-save]').onclick = function() {
    try {
      var md = crepe.getMarkdown();
      var html = crepe.editor.action(getHTML());
      hiddenInput.value = JSON.stringify({ markdown: md, html: html });
      var snippet = field.querySelector('[data-richtext-snippet]');
      if (snippet) {
        var text = md.replace(/[#*_~`>\[\]()!]/g, '').trim();
        snippet.textContent = text.length > 200 ? text.slice(0, 200) + '...' : (text || 'No content yet');
      }
    } catch (err) {
      console.error('richtext save error:', err);
    }
    close();
  };

  modal.querySelector('[data-richtext-discard]').onclick = close;

  overlay.onclick = function(e) {
    if (e.target === overlay) close();
  };
}

function initRichtextEditors() {
  document.querySelectorAll('[data-richtext]:not(.richtext-init)').forEach(function(field) {
    field.classList.add('richtext-init');
    var preview = field.querySelector('[data-richtext-preview]');
    if (preview) {
      preview.addEventListener('click', function(e) {
        if (e.target.closest('[data-richtext-open]')) return;
        openEditor(field);
      });
    }
    var btn = field.querySelector('[data-richtext-open]');
    if (btn) {
      btn.addEventListener('click', function() {
        openEditor(field);
      });
    }
  });
}

window.initRichtextEditors = initRichtextEditors;

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initRichtextEditors);
} else {
  initRichtextEditors();
}
