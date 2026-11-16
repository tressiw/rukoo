'use strict';

// Where the title bar's content sits against the native window buttons on Windows and Linux. The page learns where
// they are from navigator.windowControlsOverlay: its titlebar-area rectangle is the part of the bar they leave free,
// in CSS pixels at the page's zoom.

// The free part of the bar, the window's width, the side bootstrap.js marked for the buttons, and the visible brand
// icon, compose title, search and actions in the bar.
function titlebar(page) {
  return page.evaluate(() => {
    const area = navigator.windowControlsOverlay.getTitlebarAreaRect();
    const box = (selector) => {
      const el = [...document.querySelectorAll(selector)].find((e) => e.checkVisibility());
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { left: r.left, right: r.right };
    };
    return {
      width: innerWidth,
      controls: document.documentElement.dataset.controls ?? null,
      area: { left: area.x, right: area.x + area.width, height: area.height },
      icon: box('.titlebar-icon'),
      title: box('#compose-title'),
      search: box('.titlebar-search'),
      actions: box('.titlebar-actions')
    };
  });
}

// How far the icon starts after the buttons on the left (or the window's edge), and how far the actions end before
// the buttons on the right (or the edge), rounded to whole CSS pixels. Null for a part the page doesn't show.
async function clearance(page) {
  const g = await titlebar(page);
  return {
    icon: g.icon && Math.round(g.icon.left - g.area.left),
    actions: g.actions && Math.round(g.area.right - g.actions.right)
  };
}

// The title bar with the window buttons on the right, measured from the window's edges as main has always laid it
// out: the icon 22px from the left edge (16px in a compose window) and the actions 150px from the right edge.
async function edges(page) {
  const g = await titlebar(page);
  return {
    controls: g.controls,
    icon: g.icon && Math.round(g.icon.left),
    actions: g.actions && Math.round(g.width - g.actions.right)
  };
}

module.exports = { titlebar, clearance, edges };
