/* ==========================================================================
   pv — Landing Page Scripts
   Terminal typewriter animation + copy-to-clipboard
   ========================================================================== */

(function () {
  'use strict';

  // --------------------------------------------------------------------------
  // Terminal Typewriter Animation
  // --------------------------------------------------------------------------

  const sequences = [
    {
      command: 'pv install',
      output: [
        '\u2713 FrankenPHP installed',
        '\u2713 PHP 8.4 ready',
        '\u2713 Composer installed',
      ],
      pauseAfter: 1800,
    },
    {
      command: 'pv link ~/code/my-app',
      output: [
        '\u2713 Linked my-app',
        '\u2713 Starting server...',
        '\u2713 Live at https://my-app.test',
      ],
      pauseAfter: 2200,
    },
    {
      command: 'pv php install 8.3',
      output: [
        '\u2713 Downloading PHP 8.3...',
        '\u2713 PHP 8.3 installed',
      ],
      pauseAfter: 1800,
    },
  ];

  const TYPING_SPEED = 45;       // ms per character
  const OUTPUT_LINE_DELAY = 200; // ms between output lines
  const PAUSE_BEFORE_CLEAR = 600;

  function sleep(ms) {
    return new Promise(function (resolve) { setTimeout(resolve, ms); });
  }

  function initTerminal() {
    var body = document.getElementById('terminal-body');
    if (!body) return;

    var seqIndex = 0;

    async function runSequence() {
      while (true) {
        var seq = sequences[seqIndex];
        body.innerHTML = '';

        // Create prompt line with cursor
        var promptLine = document.createElement('span');
        promptLine.className = 'terminal__line terminal__line--prompt';
        body.appendChild(promptLine);

        var cursor = document.createElement('span');
        cursor.className = 'terminal__cursor';
        body.appendChild(cursor);

        // Type out the command character by character
        for (var i = 0; i < seq.command.length; i++) {
          promptLine.textContent += seq.command[i];
          await sleep(TYPING_SPEED);
        }

        await sleep(300);

        // Remove cursor temporarily
        cursor.remove();

        // Print output lines one by one
        for (var j = 0; j < seq.output.length; j++) {
          var outputLine = document.createElement('span');
          outputLine.className = 'terminal__line terminal__line--output';
          outputLine.textContent = seq.output[j];
          body.appendChild(outputLine);
          await sleep(OUTPUT_LINE_DELAY);
        }

        // Add blinking cursor on new prompt line
        var newPrompt = document.createElement('span');
        newPrompt.className = 'terminal__line terminal__line--prompt';
        body.appendChild(newPrompt);

        var newCursor = document.createElement('span');
        newCursor.className = 'terminal__cursor';
        body.appendChild(newCursor);

        // Pause to let user read
        await sleep(seq.pauseAfter);

        // Move to next sequence
        seqIndex = (seqIndex + 1) % sequences.length;
        await sleep(PAUSE_BEFORE_CLEAR);
      }
    }

    runSequence();
  }

  // --------------------------------------------------------------------------
  // Copy to Clipboard
  // --------------------------------------------------------------------------

  function initCopyButton() {
    var btn = document.getElementById('copy-btn');
    if (!btn) return;

    var command = 'curl -fsSL https://raw.githubusercontent.com/prvious/pv/main/install.sh | bash';

    var copyIcon = '<svg viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>';
    var checkIcon = '<svg viewBox="0 0 24 24"><polyline points="20 6 9 17 4 12"></polyline></svg>';

    btn.addEventListener('click', function () {
      // Clipboard API requires secure context (HTTPS or localhost)
      // Falls back gracefully if unavailable
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(command).then(function () {
          btn.innerHTML = checkIcon;
          btn.classList.add('install__copy--copied');
          setTimeout(function () {
            btn.innerHTML = copyIcon;
            btn.classList.remove('install__copy--copied');
          }, 2000);
        });
      } else {
        // Fallback: select text from a temporary textarea
        var ta = document.createElement('textarea');
        ta.value = command;
        ta.style.position = 'fixed';
        ta.style.left = '-9999px';
        document.body.appendChild(ta);
        ta.select();
        try {
          document.execCommand('copy');
          btn.innerHTML = checkIcon;
          btn.classList.add('install__copy--copied');
          setTimeout(function () {
            btn.innerHTML = copyIcon;
            btn.classList.remove('install__copy--copied');
          }, 2000);
        } catch (e) {
          // silently fail
        }
        document.body.removeChild(ta);
      }
    });
  }

  // --------------------------------------------------------------------------
  // Scroll Fade-In Animation
  // --------------------------------------------------------------------------

  function initScrollAnimations() {
    var elements = document.querySelectorAll('.fade-in');
    if (!elements.length) return;

    var observer = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add('visible');
          observer.unobserve(entry.target);
        }
      });
    }, {
      threshold: 0.1,
      rootMargin: '0px 0px -40px 0px',
    });

    elements.forEach(function (el) {
      observer.observe(el);
    });
  }

  // --------------------------------------------------------------------------
  // Smooth Scroll for CTA
  // --------------------------------------------------------------------------

  function initSmoothScroll() {
    var links = document.querySelectorAll('a[href^="#"]');
    links.forEach(function (link) {
      link.addEventListener('click', function (e) {
        var targetId = link.getAttribute('href');
        if (targetId === '#') return;
        var target = document.querySelector(targetId);
        if (target) {
          e.preventDefault();
          target.scrollIntoView({ behavior: 'smooth', block: 'start' });
        }
      });
    });
  }

  // --------------------------------------------------------------------------
  // Color Palette Switcher
  // --------------------------------------------------------------------------

  var palettes = [
    {
      name: 'Electric Blue',
      accent: '#155dfc',
      accentHover: '#3b82f6',
      swatch: '#155dfc',
    },
    {
      name: 'Emerald',
      accent: '#10b981',
      accentHover: '#34d399',
      swatch: '#10b981',
    },
    {
      name: 'Violet',
      accent: '#8b5cf6',
      accentHover: '#a78bfa',
      swatch: '#8b5cf6',
    },
    {
      name: 'Rose',
      accent: '#f43f5e',
      accentHover: '#fb7185',
      swatch: '#f43f5e',
    },
    {
      name: 'Amber',
      accent: '#f59e0b',
      accentHover: '#fbbf24',
      swatch: '#f59e0b',
    },
    {
      name: 'Cyan',
      accent: '#06b6d4',
      accentHover: '#22d3ee',
      swatch: '#06b6d4',
    },
    {
      name: 'Orange',
      accent: '#f97316',
      accentHover: '#fb923c',
      swatch: '#f97316',
    },
    {
      name: 'Pink',
      accent: '#ec4899',
      accentHover: '#f472b6',
      swatch: '#ec4899',
    },
    {
      name: 'Teal',
      accent: '#14b8a6',
      accentHover: '#2dd4bf',
      swatch: '#14b8a6',
    },
    {
      name: 'Indigo',
      accent: '#6366f1',
      accentHover: '#818cf8',
      swatch: '#6366f1',
    },
  ];

  // Convert hex to RGB components
  function hexToRgb(hex) {
    var r = parseInt(hex.slice(1, 3), 16);
    var g = parseInt(hex.slice(3, 5), 16);
    var b = parseInt(hex.slice(5, 7), 16);
    return { r: r, g: g, b: b };
  }

  function applyPalette(index) {
    var p = palettes[index];
    var root = document.documentElement;
    var rgb = hexToRgb(p.accent);
    var r = rgb.r, g = rgb.g, b = rgb.b;

    root.style.setProperty('--accent', p.accent);
    root.style.setProperty('--accent-hover', p.accentHover);
    root.style.setProperty('--accent-glow', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.25)');
    root.style.setProperty('--accent-bg-subtle', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.1)');
    root.style.setProperty('--accent-bg-medium', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.12)');
    root.style.setProperty('--accent-border', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.25)');
    root.style.setProperty('--accent-border-strong', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.3)');
    root.style.setProperty('--accent-radial', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.18)');
    root.style.setProperty('--accent-card-glow', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.08)');
    root.style.setProperty('--accent-code-bg', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.12)');
    root.style.setProperty('--accent-code-border', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.2)');
    root.style.setProperty('--border-hover', 'rgba(' + r + ', ' + g + ', ' + b + ', 0.4)');
  }

  function initColorSwitcher() {
    var currentIndex = 0;

    // Build the widget
    var widget = document.createElement('div');
    widget.className = 'color-switcher';

    // Swatch (the clickable circle)
    var swatch = document.createElement('button');
    swatch.className = 'color-switcher__swatch';
    swatch.style.background = palettes[0].swatch;
    swatch.setAttribute('aria-label', 'Switch color palette');
    swatch.title = palettes[0].name;

    // Label
    var label = document.createElement('span');
    label.className = 'color-switcher__label';
    label.textContent = palettes[0].name;

    widget.appendChild(label);
    widget.appendChild(swatch);
    document.body.appendChild(widget);

    swatch.addEventListener('click', function () {
      currentIndex = (currentIndex + 1) % palettes.length;
      applyPalette(currentIndex);

      // Update swatch color and label
      swatch.style.background = palettes[currentIndex].swatch;
      swatch.title = palettes[currentIndex].name;
      label.textContent = palettes[currentIndex].name;

      // Flash the label visible
      label.classList.remove('color-switcher__label--visible');
      // Force reflow
      void label.offsetWidth;
      label.classList.add('color-switcher__label--visible');
    });
  }

  // --------------------------------------------------------------------------
  // Init
  // --------------------------------------------------------------------------

  document.addEventListener('DOMContentLoaded', function () {
    initTerminal();
    initCopyButton();
    initScrollAnimations();
    initSmoothScroll();
    initColorSwitcher();
  });
})();
