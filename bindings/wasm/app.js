"use strict";

// The page in one file: a reader for the `.vtree` format, a layout and a
// canvas drawing of the tree, a viewer for the bundle's text files, a
// store-only zip writer, and the calls into the worker that runs vitri.
// Nothing is loaded from anywhere else.

// ------------------------------------------------------------------ limits

// Nodes drawn when a tree first appears. Deeper subtrees start collapsed, so
// a tree of any size opens as quickly as a small one.
const INITIAL_NODES = 1023;
// Nodes one "Expand below" adds at most.
const EXPAND_NODES = 40000;
// An input past this size stays out of the text box, which becomes slow on
// multi-megabyte text; it is held in memory and sent as it is.
const HELD_INPUT_BYTES = 1024 * 1024;
// A line of a shown file is cut at this many characters on screen; Save
// writes it whole.
const SHOWN_LINE_CHARS = 2000;
// Browsers cap the height of an element; past this the file view maps its
// scroll position onto the lines instead of giving each line its own pixels.
const MAX_SCROLL_HEIGHT = 8e6;
// Items in the list form of the drawn tree.
const OUTLINE_ITEMS = 2000;
// Leaf variables listed for an internal node.
const LISTED_LEAVES = 24;
// The largest PNG side and area the export draws; browsers refuse larger
// canvases.
const PNG_SIDE = 16384;
const PNG_AREA = 1.2e8;

// ----------------------------------------------------------------- reading

const isCount = (token) => /^\+?\d+$/.test(token);

// The `.vtree` text format the SDD library reads: a `vtree N` line, then N
// node lines, `L <id> <variable>` or `I <id> <left> <right>`, children before
// parents. The last node listed is the root. The checks are the crate's own:
// ids in range, every node listed, no variable on two leaves, and one tree.
//
// Nodes live in typed arrays indexed by the file's node id. `order` is a
// preorder walk, left child first; `pre[v]` is v's position in it and
// `last[v]` the position of the last node of v's subtree, so u is under v
// exactly when pre[v] <= pre[u] <= last[v].
function parseVtree(text) {
  const lines = text.split("\n");
  const header = (lines[0] ?? "").replace(/\r$/, "");
  if (header.trim() === "") return { error: "the vtree file is empty" };
  const match = /^vtree (\s*\+?\d+\s*)$/.exec(header);
  if (match === null) return { error: "the first line is not 'vtree N'" };
  const count = Number(match[1].trim());
  if (count < 1) return { error: "the header declares no nodes; a vtree has at least one" };
  if (!Number.isSafeInteger(count) || count > 1 << 27) {
    return { error: `the header declares ${match[1].trim()} nodes, more than this page reads` };
  }

  const left = new Int32Array(count).fill(-1);
  const right = new Int32Array(count).fill(-1);
  const variable = new Int32Array(count);
  const listed = new Uint8Array(count);
  const leafOf = new Map();
  let root = -1;
  let maxVariable = 0;
  const inRange = (what, token, line) => {
    if (!isCount(token)) return `line ${line}: ${what} ${token} is not a number`;
    const value = Number(token);
    if (value >= count) return `line ${line}: ${what} ${value} is out of range for the ${count} nodes the header declares`;
    return value;
  };

  for (let i = 1; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "") continue;
    const fields = line.split(/\s+/);
    if (fields[0] === "L") {
      if (fields.length !== 3) return { error: `line ${i + 1} is not a leaf line` };
      const id = inRange("node id", fields[1], i + 1);
      if (typeof id === "string") return { error: id };
      if (!isCount(fields[2])) return { error: `line ${i + 1}: variable ${fields[2]} is not a number` };
      const v = Number(fields[2]);
      if (v === 0) return { error: `line ${i + 1}: leaf ${id} names variable 0; vtree variables are 1-based` };
      if (v > 0x7fffffff) return { error: `line ${i + 1}: variable ${v} is too large` };
      if (leafOf.has(v)) {
        return { error: `leaves ${leafOf.get(v)} and ${id} both name variable ${v}; a vtree carries each variable on one leaf` };
      }
      leafOf.set(v, id);
      variable[id] = v;
      left[id] = -1;
      right[id] = -1;
      maxVariable = Math.max(maxVariable, v);
      listed[id] = 1;
      root = id;
    } else if (fields[0] === "I") {
      if (fields.length !== 4) return { error: `line ${i + 1} is not an internal line` };
      const id = inRange("node id", fields[1], i + 1);
      if (typeof id === "string") return { error: id };
      const l = inRange("left child", fields[2], i + 1);
      if (typeof l === "string") return { error: l };
      const r = inRange("right child", fields[3], i + 1);
      if (typeof r === "string") return { error: r };
      variable[id] = 0;
      left[id] = l;
      right[id] = r;
      listed[id] = 1;
      root = id;
    } else {
      return { error: `line ${i + 1} is neither a leaf nor an internal line` };
    }
  }
  for (let id = 0; id < count; id++) {
    if (listed[id] === 0) return { error: `node ${id} is declared but not listed` };
  }

  // One walk from the root reaching every node exactly once is what makes
  // the lines a tree.
  const parent = new Int32Array(count).fill(-1);
  const order = new Int32Array(count);
  const stack = new Int32Array(count);
  const seen = new Uint8Array(count);
  let top = 0;
  let reached = 0;
  stack[top++] = root;
  seen[root] = 1;
  while (top > 0) {
    const node = stack[--top];
    order[reached++] = node;
    if (left[node] < 0) continue;
    // Right first, so the left subtree comes first in the walk.
    for (const child of [right[node], left[node]]) {
      if (seen[child] === 1) return { error: `node ${child} is reachable twice; the lines are not a tree` };
      seen[child] = 1;
      parent[child] = node;
      stack[top++] = child;
    }
  }
  if (reached !== count) {
    return { error: `${count - reached} of the ${count} declared nodes are unreachable from the root` };
  }

  const pre = new Int32Array(count);
  const last = new Int32Array(count);
  const leaves = new Int32Array(count);
  const depth = new Int32Array(count);
  let height = 0;
  for (let i = 0; i < count; i++) {
    const node = order[i];
    pre[node] = i;
    if (parent[node] >= 0) {
      depth[node] = depth[parent[node]] + 1;
      if (depth[node] > height) height = depth[node];
    }
  }
  for (let i = count - 1; i >= 0; i--) {
    const node = order[i];
    if (left[node] < 0) {
      leaves[node] = 1;
      last[node] = i;
    } else {
      leaves[node] = leaves[left[node]] + leaves[right[node]];
      last[node] = last[right[node]];
    }
  }
  return { count, root, left, right, variable, parent, order, pre, last, leaves, depth, height, maxVariable };
}

const isLeaf = (tree, node) => tree.left[node] < 0;
const isUnder = (tree, node, top) => tree.pre[top] <= tree.pre[node] && tree.pre[node] <= tree.last[top];

// Up to `limit` leaf variables of a subtree, left to right, and whether that
// was all of them.
function leafVariables(tree, top, limit) {
  const found = [];
  const stack = [top];
  while (stack.length > 0 && found.length < limit) {
    const node = stack.pop();
    if (isLeaf(tree, node)) found.push(tree.variable[node]);
    else stack.push(tree.right[node], tree.left[node]);
  }
  return { variables: found, all: tree.leaves[top] <= limit };
}

// The `p cnf <variables> <clauses>` line of a DIMACS text, from its first
// lines alone, or null.
function dimacsHeader(text) {
  const match = /^p\s+cnf\s+(\d+)\s+(\d+)/m.exec(text.slice(0, 1 << 20));
  return match === null ? null : { variables: Number(match[1]), clauses: Number(match[2]) };
}

// How many clauses of a DIMACS text each variable occurs in, indexed by
// variable. A variable twice in one clause counts once.
function occurrences(text, variables) {
  const counts = new Int32Array(variables + 1);
  const inClause = new Int32Array(variables + 1);
  let clause = 1;
  let at = 0;
  while (at < text.length) {
    let end = text.indexOf("\n", at);
    if (end === -1) end = text.length;
    const first = text.charCodeAt(at);
    // Comment, problem and weight lines, and the SATLIB end marker.
    if (first !== 99 && first !== 112 && first !== 119 && first !== 37) {
      const tokens = text.slice(at, end).trim().split(/\s+/);
      for (const token of tokens) {
        if (token === "") continue;
        const literal = Math.abs(Number(token));
        if (literal === 0) {
          clause += 1;
        } else if (literal <= variables && inClause[literal] !== clause) {
          inClause[literal] = clause;
          counts[literal] += 1;
        }
      }
    }
    at = end + 1;
  }
  return counts;
}

// -------------------------------------------------------------------- zip

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes) {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// A zip archive of the given files, stored without compression: a local
// header and the bytes per file, then the central directory and its end
// record. Names are UTF-8 (flag bit 11). Zip64 is not written, so the whole
// archive stays under 4 GiB and 65,535 files.
function zipStore(entries, date = new Date()) {
  const encoder = new TextEncoder();
  const year = Math.max(1980, date.getFullYear());
  const time = (date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1);
  const day = ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
  const files = entries.map(({ name, data }) => {
    const bytes = typeof data === "string" ? encoder.encode(data) : data;
    return { name: encoder.encode(name), bytes, crc: crc32(bytes) };
  });
  if (files.length > 0xffff) throw new Error("more files than a zip without Zip64 holds");
  let size = 22;
  for (const file of files) size += 30 + file.name.length + file.bytes.length + 46 + file.name.length;
  if (size > 0xffffffff) throw new Error("the bundle is larger than a zip without Zip64 holds");

  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let at = 0;
  const u16 = (value) => {
    view.setUint16(at, value, true);
    at += 2;
  };
  const u32 = (value) => {
    view.setUint32(at, value, true);
    at += 4;
  };
  const offsets = [];
  for (const file of files) {
    offsets.push(at);
    u32(0x04034b50);
    u16(20);
    u16(0x0800);
    u16(0);
    u16(time);
    u16(day);
    u32(file.crc);
    u32(file.bytes.length);
    u32(file.bytes.length);
    u16(file.name.length);
    u16(0);
    out.set(file.name, at);
    at += file.name.length;
    out.set(file.bytes, at);
    at += file.bytes.length;
  }
  const directory = at;
  files.forEach((file, i) => {
    u32(0x02014b50);
    u16(20);
    u16(20);
    u16(0x0800);
    u16(0);
    u16(time);
    u16(day);
    u32(file.crc);
    u32(file.bytes.length);
    u32(file.bytes.length);
    u16(file.name.length);
    u16(0);
    u16(0);
    u16(0);
    u16(0);
    u32(0);
    u32(offsets[i]);
    out.set(file.name, at);
    at += file.name.length;
  });
  const directorySize = at - directory;
  u32(0x06054b50);
  u16(0);
  u16(0);
  u16(files.length);
  u16(files.length);
  u32(directorySize);
  u32(directory);
  u16(0);
  return out;
}

// ------------------------------------------------------------------ layout

// World units: one row per depth, one slot per drawn leaf or collapsed
// subtree. Labels are set in a 12px monospace, about 7.2 units a character.
const ROW = 64;
const NODE_RADIUS = 6;
const LEAF_HEIGHT = 20;
const CHARACTER = 7.2;
const STUB_HEIGHT = 26;

function sizesFor(tree) {
  const leaf = Math.max(24, String(tree.maxVariable).length * CHARACTER + 12);
  const slot = leaf + 8;
  const stub = Math.max(slot, String(tree.count).length * 6.2 + 20);
  return { leaf, slot, stub };
}

// Which internal nodes start expanded: breadth first from the root until the
// drawing would pass `budget` nodes, so the top of the tree shows evenly.
function initialExpansion(tree, budget) {
  const expanded = new Uint8Array(tree.count);
  const queue = new Int32Array(tree.count);
  let tail = 0;
  let drawn = 1;
  queue[tail++] = tree.root;
  for (let head = 0; head < tail; head++) {
    const node = queue[head];
    if (isLeaf(tree, node)) continue;
    if (drawn + 2 > budget) break;
    expanded[node] = 1;
    drawn += 2;
    queue[tail++] = tree.left[node];
    queue[tail++] = tree.right[node];
  }
  return expanded;
}

// Expands collapsed nodes under `top`, breadth first, adding at most `budget`
// drawn nodes. Returns how many it added and whether anything under `top`
// is still collapsed.
function expandBelow(tree, expanded, top, budget) {
  const queue = [top];
  let added = 0;
  let stopped = false;
  for (let head = 0; head < queue.length; head++) {
    const node = queue[head];
    if (isLeaf(tree, node)) continue;
    if (expanded[node] === 0) {
      if (added + 2 > budget) {
        stopped = true;
        break;
      }
      expanded[node] = 1;
      added += 2;
    }
    queue.push(tree.left[node], tree.right[node]);
  }
  return { added, stopped };
}

// The drawn nodes and their places. A drawn leaf or collapsed subtree takes
// the next slot from the left; an expanded node sits midway over its two
// children. `rows` lists the drawn nodes of each depth left to right, for
// finding the node under the pointer.
function layoutTree(tree, expanded, sizes, x) {
  const drawnNodes = new Int32Array(tree.count);
  const stack = new Int32Array(tree.count + 1);
  let drawn = 0;
  let top = 0;
  let cursor = 0;
  let deepest = 0;
  let collapsed = 0;
  stack[top++] = tree.root;
  while (top > 0) {
    const node = stack[--top];
    drawnNodes[drawn++] = node;
    if (tree.depth[node] > deepest) deepest = tree.depth[node];
    if (!isLeaf(tree, node) && expanded[node] === 1) {
      stack[top++] = tree.right[node];
      stack[top++] = tree.left[node];
    } else {
      const width = isLeaf(tree, node) ? sizes.slot : sizes.stub;
      if (!isLeaf(tree, node)) collapsed += 1;
      x[node] = cursor + width / 2;
      cursor += width;
    }
  }
  for (let i = drawn - 1; i >= 0; i--) {
    const node = drawnNodes[i];
    if (!isLeaf(tree, node) && expanded[node] === 1) x[node] = (x[tree.left[node]] + x[tree.right[node]]) / 2;
  }
  const rowStart = new Int32Array(deepest + 2);
  for (let i = 0; i < drawn; i++) rowStart[tree.depth[drawnNodes[i]] + 1] += 1;
  for (let d = 0; d <= deepest; d++) rowStart[d + 1] += rowStart[d];
  const fill = rowStart.slice();
  const rows = new Int32Array(drawn);
  for (let i = 0; i < drawn; i++) rows[fill[tree.depth[drawnNodes[i]]]++] = drawnNodes[i];
  return {
    nodes: drawnNodes.subarray(0, drawn),
    drawn,
    collapsed,
    deepest,
    rowStart,
    rows,
    width: cursor,
    height: deepest * ROW + STUB_HEIGHT + LEAF_HEIGHT,
  };
}

// The drawn node nearest a world point within reach of it, or -1.
function nodeAt(tree, layout, x, wx, wy, reach) {
  const depth = Math.round(wy / ROW);
  if (depth < 0 || depth > layout.deepest) return -1;
  const dy = wy - depth * ROW;
  if (dy < -LEAF_HEIGHT / 2 - reach || dy > STUB_HEIGHT + 6 + reach) return -1;
  let low = layout.rowStart[depth];
  let high = layout.rowStart[depth + 1];
  const end = high;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (x[layout.rows[mid]] < wx) low = mid + 1;
    else high = mid;
  }
  let best = -1;
  let bestDistance = Infinity;
  for (const i of [low - 1, low]) {
    if (i < layout.rowStart[depth] || i >= end) continue;
    const node = layout.rows[i];
    const distance = Math.abs(x[node] - wx);
    if (distance < bestDistance) {
      best = node;
      bestDistance = distance;
    }
  }
  return bestDistance <= 16 + reach ? best : -1;
}

// ------------------------------------------------------------------ drawing

// Draws the laid-out tree on a 2D context whose transform the caller has set
// to world units. `scene` holds the tree, its layout and places, the
// component of every node (or null) and the selection; `bounds` is the world
// rectangle to draw, and `scale` the pixels per world unit, which decides how
// much detail is legible.
function drawScene(context, scene, colors, bounds, scale) {
  const { tree, layout, x, sizes, component, selected, onPath } = scene;
  const px = 1 / scale;
  const shapes = sizes.slot * scale >= 5;
  const labels = scale >= 0.55;
  const { x0, y0, x1, y1 } = bounds;

  // Edges: from a parent down half a row, across, and down to the child. A
  // subtree narrower than a pixel is drawn as the edge into it alone; its
  // own edges would all land on that pixel. The drawn nodes are in preorder,
  // so such a subtree is the run of nodes after its root.
  const edge = (target, node) => {
    const up = tree.parent[node];
    const xc = x[node];
    const xp = x[up];
    const yc = tree.depth[node] * ROW;
    const yp = yc - ROW;
    target.moveTo(xp, yp);
    target.lineTo(xp, yp + ROW / 2);
    target.lineTo(xc, yp + ROW / 2);
    target.lineTo(xc, yc);
  };
  const plain = new Path2D();
  for (let i = 0; i < layout.drawn; i++) {
    const node = layout.nodes[i];
    const up = tree.parent[node];
    if (up >= 0) {
      const xc = x[node];
      const xp = x[up];
      const yc = tree.depth[node] * ROW;
      if (!(Math.max(xc, xp) < x0 || Math.min(xc, xp) > x1 || yc < y0 || yc - ROW > y1)) edge(plain, node);
    }
    if (!isLeaf(tree, node) && tree.leaves[node] * sizes.slot * scale < 1) {
      while (i + 1 < layout.drawn && isUnder(tree, layout.nodes[i + 1], node)) i += 1;
    }
  }
  context.lineJoin = "round";
  context.strokeStyle = colors.edge;
  context.lineWidth = Math.max(1.2, 1.2 * px);
  context.stroke(plain);
  if (selected >= 0 && onPath !== null) {
    const path = new Path2D();
    for (let node = selected; tree.parent[node] >= 0; node = tree.parent[node]) edge(path, node);
    context.strokeStyle = colors.pick;
    context.lineWidth = Math.max(3, 3 * px);
    context.stroke(path);
  }
  if (!shapes) {
    drawSelection(context, scene, colors, px, shapes);
    return;
  }

  const colorOf = (node) => {
    if (component === null) return colors.accent;
    const c = component[node];
    if (c >= 0) return colors.components[c % colors.components.length];
    return colors.muted;
  };
  context.font = `12px ${colors.mono}`;
  context.textAlign = "center";
  context.textBaseline = "middle";
  const expandedFlags = scene.expanded;
  for (let i = 0; i < layout.drawn; i++) {
    const node = layout.nodes[i];
    const cx = x[node];
    const cy = tree.depth[node] * ROW;
    if (cx < x0 - sizes.stub || cx > x1 + sizes.stub || cy < y0 - STUB_HEIGHT - LEAF_HEIGHT || cy > y1 + LEAF_HEIGHT) continue;
    const color = colorOf(node);
    if (isLeaf(tree, node)) {
      const w = sizes.leaf;
      context.beginPath();
      roundedRect(context, cx - w / 2, cy - LEAF_HEIGHT / 2, w, LEAF_HEIGHT, 4);
      context.fillStyle = colors.card;
      context.fill();
      context.lineWidth = Math.max(1.4, 1.2 * px);
      context.strokeStyle = color;
      if (component !== null && component[node] === -2) context.setLineDash([3, 2]);
      context.stroke();
      context.setLineDash([]);
      if (labels) {
        context.fillStyle = colors.ink;
        context.fillText(String(tree.variable[node]), cx, cy + 0.5);
      }
    } else {
      if (expandedFlags[node] === 0) {
        const half = sizes.stub / 2 - 5;
        context.beginPath();
        context.moveTo(cx, cy);
        context.lineTo(cx - half, cy + STUB_HEIGHT);
        context.lineTo(cx + half, cy + STUB_HEIGHT);
        context.closePath();
        context.globalAlpha = 0.18;
        context.fillStyle = color;
        context.fill();
        context.globalAlpha = 1;
        context.lineWidth = Math.max(1.2, 1.2 * px);
        context.strokeStyle = color;
        context.stroke();
        if (labels) {
          context.font = `10px ${colors.mono}`;
          context.fillStyle = colors.ink;
          context.fillText(String(tree.leaves[node]), cx, cy + STUB_HEIGHT - 7);
          context.font = `12px ${colors.mono}`;
        }
      }
      context.beginPath();
      context.arc(cx, cy, NODE_RADIUS, 0, 2 * Math.PI);
      context.fillStyle = color;
      context.fill();
    }
  }
  drawSelection(context, scene, colors, px, shapes);
}

function drawSelection(context, scene, colors, px, shapes) {
  const { tree, x, sizes, selected } = scene;
  if (selected < 0) return;
  const cx = x[selected];
  const cy = tree.depth[selected] * ROW;
  context.lineWidth = 3 * px * (shapes ? 1 : 1.5);
  context.strokeStyle = colors.pick;
  context.beginPath();
  if (isLeaf(tree, selected)) {
    const w = shapes ? sizes.leaf + 8 : 14 * px;
    const h = shapes ? LEAF_HEIGHT + 8 : 14 * px;
    roundedRect(context, cx - w / 2, cy - h / 2, w, h, shapes ? 6 : 0);
  } else {
    context.arc(cx, cy, shapes ? NODE_RADIUS + 5 : 8 * px, 0, 2 * Math.PI);
  }
  context.stroke();
}

function roundedRect(context, left, top, width, height, radius) {
  if (typeof context.roundRect === "function") context.roundRect(left, top, width, height, radius);
  else context.rect(left, top, width, height);
}

// The same drawing as an SVG document: every drawn node, labels included.
function sceneSvg(scene, colors, title) {
  const { tree, layout, x, sizes, component, onPath } = scene;
  const ns = "http://www.w3.org/2000/svg";
  const pad = 20;
  const width = Math.ceil(layout.width + 2 * pad);
  const height = Math.ceil(layout.height + 2 * pad);
  const make = (name, attributes, parent) => {
    const element = document.createElementNS(ns, name);
    for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, String(value));
    if (parent) parent.append(element);
    return element;
  };
  const r = (value) => Math.round(value * 100) / 100;
  const svg = make("svg", { xmlns: ns, width, height, viewBox: `0 0 ${width} ${height}` });
  make("title", {}, svg).textContent = title;
  make("rect", { width, height, fill: colors.card }, svg);
  const world = make("g", { transform: `translate(${pad} ${pad + LEAF_HEIGHT / 2})` }, svg);
  let plain = "";
  let path = "";
  for (let i = 0; i < layout.drawn; i++) {
    const node = layout.nodes[i];
    const up = tree.parent[node];
    if (up < 0) continue;
    const yc = tree.depth[node] * ROW;
    const d = `M${r(x[up])} ${yc - ROW}V${yc - ROW / 2}H${r(x[node])}V${yc}`;
    if (onPath !== null && onPath[node] === 1) path += d;
    else plain += d;
  }
  make("path", { d: plain || "M0 0", fill: "none", stroke: colors.edge, "stroke-width": 1.2 }, world);
  if (path !== "") make("path", { d: path, fill: "none", stroke: colors.pick, "stroke-width": 3 }, world);
  const colorOf = (node) => {
    if (component === null) return colors.accent;
    const c = component[node];
    return c >= 0 ? colors.components[c % colors.components.length] : colors.muted;
  };
  const font = { "font-family": colors.mono, "font-size": 12, "text-anchor": "middle", "dominant-baseline": "central" };
  for (let i = 0; i < layout.drawn; i++) {
    const node = layout.nodes[i];
    const cx = r(x[node]);
    const cy = tree.depth[node] * ROW;
    const color = colorOf(node);
    if (isLeaf(tree, node)) {
      const group = make("g", {}, world);
      make("rect", {
        x: r(cx - sizes.leaf / 2), y: cy - LEAF_HEIGHT / 2, width: r(sizes.leaf), height: LEAF_HEIGHT, rx: 4,
        fill: colors.card, stroke: color, "stroke-width": 1.4,
        ...(component !== null && component[node] === -2 ? { "stroke-dasharray": "3 2" } : {}),
      }, group);
      make("text", { x: cx, y: cy, fill: colors.ink, ...font }, group).textContent = String(tree.variable[node]);
    } else {
      if (scene.expanded[node] === 0) {
        const half = sizes.stub / 2 - 5;
        make("path", {
          d: `M${cx} ${cy}L${r(cx - half)} ${cy + STUB_HEIGHT}H${r(cx + half)}Z`,
          fill: color, "fill-opacity": 0.18, stroke: color, "stroke-width": 1.2,
        }, world);
        make("text", { x: cx, y: cy + STUB_HEIGHT - 7, fill: colors.ink, ...font, "font-size": 10 }, world).textContent =
          String(tree.leaves[node]);
      }
      make("circle", { cx, cy, r: NODE_RADIUS, fill: color }, world);
    }
  }
  return new XMLSerializer().serializeToString(svg);
}

// ----------------------------------------------------------------- examples

// The row of examples: a key, which is how the address bar names one, the
// name on its button, and its maker. The tutorial formula and the competition
// formula are files beside the page; the others are made from the tutorial
// formula or written out here.
const EXAMPLES = [
  ["choices", "tutorial formula, 12 variables", () => fetched("example.cnf")],
  ["choices-projected", "projected on the first slot", () => fetched("example.cnf").then(projectedChoices)],
  ["choices-weighted", "weighted", () => fetched("example.cnf").then(weightedChoices)],
  ["three-copies", "three independent copies", () => fetched("example.cnf").then(threeCopies)],
  ["unsatisfiable", "unsatisfiable", () => UNSATISFIABLE],
  ["units", "unit clauses only", () => UNITS],
  ["mc2023-track1-008", "competition formula, 58 variables", () => fetched("mc2023_track1_008.reduced.cnf")],
];

const UNSATISFIABLE = [
  "c no assignment to variables 1 and 2 satisfies the first four clauses",
  "p cnf 4 5",
  "1 2 0",
  "-1 2 0",
  "1 -2 0",
  "-1 -2 0",
  "3 4 0",
  "",
].join("\n");

const UNITS = ["c every clause is a unit", "p cnf 4 4", "1 0", "-2 0", "3 0", "-4 0", ""].join("\n");

// The page's version stamp (see README.md), passed on to every file it loads.
const stamp = typeof document === "undefined" || document.currentScript === null
  ? ""
  : new URL(document.currentScript.src).search;

function fetched(file) {
  return fetch(file + stamp).then((response) => {
    if (!response.ok) throw new Error(`${response.status} for ${file}`);
    return response.text();
  });
}

// The clause lines and the declared counts of a DIMACS text.
function clausesOf(text) {
  const header = dimacsHeader(text);
  const clauses = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "" && !/^[cpw%]/.test(line));
  return { variables: header === null ? 0 : header.variables, clauses };
}

const renumbered = (clause, offset) =>
  clause
    .split(/\s+/)
    .map((token) => {
      const literal = Number(token);
      return String(literal > 0 ? literal + offset : literal < 0 ? literal - offset : 0);
    })
    .join(" ");

const dimacs = (lines) => lines.join("\n") + "\n";

// The tutorial formula counted over the first slot's four options.
function projectedChoices(text) {
  const { variables, clauses } = clausesOf(text);
  return dimacs([
    "c the tutorial formula, projected on the options of the first slot",
    "c t pmc",
    `p cnf ${variables} ${clauses.length}`,
    "c p show 1 2 3 4 0",
    ...clauses,
  ]);
}

// The tutorial formula with a weight on every literal, and one more variable
// that occurs in no clause.
function weightedChoices(text) {
  const { variables, clauses } = clausesOf(text);
  const weights = [];
  for (let v = 1; v <= variables + 1; v++) {
    weights.push(`c p weight ${v} 1/${v + 1} 0`, `c p weight -${v} ${v}/${v + 1} 0`);
  }
  return dimacs([
    "c the tutorial formula with literal weights and one unconstrained variable",
    "c t wmc",
    `p cnf ${variables + 1} ${clauses.length}`,
    ...weights,
    ...clauses,
  ]);
}

// Three copies of the tutorial formula on disjoint variables, and one more
// variable that occurs in no clause.
function threeCopies(text) {
  const { variables, clauses } = clausesOf(text);
  const lines = ["c three copies of the tutorial formula and one unconstrained variable", `p cnf ${3 * variables + 1} ${3 * clauses.length}`];
  for (let copy = 0; copy < 3; copy++) {
    for (const clause of clauses) lines.push(renumbered(clause, copy * variables));
  }
  return dimacs(lines);
}

// ------------------------------------------------------------------- words

const MODE_NAMES = {
  mc: "model counting",
  wmc: "weighted model counting",
  pmc: "projected model counting",
  pwmc: "projected weighted model counting",
  compile: "compilation, preserving the Boolean function",
};

const STAGE_NAMES = { simplify: "Simplify", arjun: "Arjun", sbva: "SBVA" };

const OUTCOMES = {
  ran: "ran",
  skipped: "skipped",
  gave_up: "gave up",
  discarded: "ran; its result was discarded",
};

const ERROR_KINDS = {
  config: "a setting was refused",
  spec: "the vtree spec was refused",
  env: "the environment was refused",
  input: "the input was refused",
  io: "a file could not be read or written",
  mismatch: "the result did not match its input",
  construction: "the vtree construction failed",
  load: "vitri did not load",
  worker: "the worker failed",
  format: "the result could not be read",
};

const PLURALS = { leaf: "leaves" };
const plural = (count, word) =>
  `${count.toLocaleString("en-US")} ${count === 1 ? word : PLURALS[word] ?? `${word}s`}`;

function megabytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

function duration(ms) {
  if (ms < 1) return "under 1 ms";
  if (ms < 10000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

// A signed literal of the original formula, from `reduced_to_original_dimacs`.
function originalLiteral(record, reduced) {
  const map = record === null ? null : record.reduced_to_original_dimacs;
  if (!Array.isArray(map)) return "unknown: preprocess.json has no variable map";
  if (reduced < 1 || reduced > map.length) return "outside the variable map";
  const literal = map[reduced - 1];
  if (literal === null) return "none: introduced by preprocessing";
  return literal > 0 ? `variable ${literal}` : `the negation of variable ${-literal}`;
}

function originalToken(record, reduced) {
  const map = record === null ? null : record.reduced_to_original_dimacs;
  if (!Array.isArray(map) || reduced < 1 || reduced > map.length) return "?";
  const literal = map[reduced - 1];
  return literal === null ? "none" : String(literal);
}

// ------------------------------------------------------------------ the page

if (typeof document !== "undefined") {
  const element = (id) => document.getElementById(id);
  const make = (name, className, text) => {
    const node = document.createElement(name);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };

  const cnfBox = element("cnf");
  const statusLine = element("status");
  const treeView = element("tree-view");
  const canvas = element("tree-canvas");
  const textView = element("text-view");

  // The input: the text box, or a large file held in memory with its name.
  let held = null;
  let inputName = null;
  const inputText = () => (held === null ? cnfBox.value : held.text);

  let capabilities = null;
  // The result on show, or null.
  let current = null;
  // The tree view on show, or null.
  let scene = null;

  // ------------------------------------------------------------- examples

  const examples = element("examples");
  EXAMPLES.forEach(([, name], i) => {
    const chip = make("button", "", name);
    chip.type = "button";
    chip.dataset.example = String(i);
    examples.append(chip);
  });
  const markExample = (i) => {
    for (const chip of examples.children) chip.classList.toggle("chosen", chip.dataset.example === String(i));
  };
  const chosenExample = () => {
    const chip = examples.querySelector(".chosen");
    return chip === null ? -1 : Number(chip.dataset.example);
  };

  let loading = 0;
  async function loadExample(i) {
    const ticket = ++loading;
    markExample(i);
    let text;
    try {
      text = await EXAMPLES[i][2]();
    } catch (failure) {
      statusLine.textContent = `The example did not load: ${failure.message ?? failure}`;
      return;
    }
    if (ticket !== loading) return;
    setInput(text, null);
    markExample(i);
    requestRun();
  }
  examples.addEventListener("click", (event) => {
    const chip = event.target.closest("button");
    if (chip !== null) loadExample(Number(chip.dataset.example));
  });

  // ---------------------------------------------------------------- input

  function setInput(text, name) {
    inputName = name;
    if (text.length > HELD_INPUT_BYTES) {
      held = { text };
      cnfBox.value = "";
      cnfBox.disabled = true;
      cnfBox.placeholder = "This file is too large to show here. It is held in memory and will be sent as it is; Clear removes it.";
      element("clear").hidden = false;
    } else {
      held = null;
      cnfBox.disabled = false;
      cnfBox.value = text;
      cnfBox.placeholder = "p cnf 3 2\n1 -2 0\n2 3 0";
      element("clear").hidden = true;
    }
    markExample(-1);
    describeInput();
    markStale();
  }

  function describeInput() {
    const text = inputText();
    const parts = [];
    if (inputName !== null) parts.push(inputName);
    if (held !== null || text.length > 64 * 1024) parts.push(megabytes(new TextEncoder().encode(text.slice(0, 1 << 24)).length + Math.max(0, text.length - (1 << 24))));
    const header = dimacsHeader(text);
    if (header !== null) parts.push(`${plural(header.variables, "variable")}, ${plural(header.clauses, "clause")} declared`);
    element("input-info").textContent = parts.join(", ");
  }

  let typing = 0;
  cnfBox.addEventListener("input", () => {
    inputName = null;
    markExample(-1);
    markStale();
    clearTimeout(typing);
    typing = setTimeout(describeInput, 300);
  });

  element("clear").addEventListener("click", () => {
    setInput("", null);
    cnfBox.focus();
  });

  // A gzip file is decompressed in the tab; other compressed formats are
  // refused by name rather than sent as bytes.
  async function readFile(file) {
    const head = new Uint8Array(await file.slice(0, 6).arrayBuffer());
    const starts = (...bytes) => bytes.every((byte, i) => head[i] === byte);
    if (starts(0x1f, 0x8b)) {
      if (typeof DecompressionStream === "undefined") throw new Error("this browser cannot decompress gzip");
      return new Response(file.stream().pipeThrough(new DecompressionStream("gzip"))).text();
    }
    if (starts(0xfd, 0x37, 0x7a, 0x58, 0x5a)) throw new Error("it is xz-compressed; decompress it first");
    if (starts(0x42, 0x5a, 0x68)) throw new Error("it is bzip2-compressed; decompress it first");
    if (starts(0x50, 0x4b, 0x03, 0x04)) throw new Error("it is a zip archive; extract the CNF first");
    return file.text();
  }

  async function loadFile(file) {
    if (file === undefined) return;
    const ticket = ++loading;
    let text;
    try {
      text = await readFile(file);
    } catch (failure) {
      statusLine.textContent = `${file.name} was not read: ${failure.message ?? failure}`;
      return;
    }
    if (ticket !== loading) return;
    setInput(text, file.name);
    requestRun();
  }

  const fileInput = element("file");
  element("open").addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    loadFile(fileInput.files[0]);
    fileInput.value = "";
  });

  // The whole page takes a drop. Dragging over child elements fires leave and
  // enter in pairs, so a count tells when the file has left the page.
  let dragging = 0;
  const hasFiles = (event) => event.dataTransfer !== null && Array.from(event.dataTransfer.types).includes("Files");
  document.addEventListener("dragenter", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragging += 1;
    document.body.classList.add("dropping");
  });
  document.addEventListener("dragover", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  });
  document.addEventListener("dragleave", (event) => {
    if (!hasFiles(event)) return;
    dragging = Math.max(0, dragging - 1);
    if (dragging === 0) document.body.classList.remove("dropping");
  });
  document.addEventListener("drop", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragging = 0;
    document.body.classList.remove("dropping");
    loadFile(event.dataTransfer.files[0]);
  });

  // ------------------------------------------------------------- settings

  const control = {
    mode: element("mode"),
    vtree: element("vtree"),
    budget: element("budget"),
    components: element("components"),
    candidates: element("candidates"),
    simplify: element("simplify"),
    arjun: element("arjun"),
  };
  // Whether this build takes a request key. A build that lists no keys is
  // taken to accept the ones this page sends.
  const accepts = (key) =>
    capabilities !== null && (!Array.isArray(capabilities.request_keys) || capabilities.request_keys.includes(key));
  const available = (stage) =>
    accepts(stage) && capabilities.stages !== undefined && capabilities.stages[stage] === true;
  // Each setting, with the request key it fills.
  const SETTING_KEYS = [
    ["mode", "mode"], ["vtree", "vtree"], ["budget", "budget_ms"], ["components", "components"],
    ["candidates", "candidates"], ["simplify", "simplify"], ["arjun", "arjun"],
  ];
  // Settings from the address, kept until the capabilities arrive.
  let addressSettings = null;

  function setUpControls() {
    const option = (select, value, text) => {
      const item = make("option", "", text);
      item.value = value;
      select.append(item);
    };
    control.mode.replaceChildren();
    option(control.mode, "", "detected from the file");
    for (const mode of capabilities.modes ?? []) option(control.mode, mode, MODE_NAMES[mode] ? `${mode}: ${MODE_NAMES[mode]}` : mode);
    const bases = element("vtree-bases");
    bases.replaceChildren();
    for (const base of capabilities.vtree_bases ?? []) option(bases, base, "");
    control.vtree.value = capabilities.default_vtree ?? "portfolio";
    control.components.replaceChildren();
    for (const policy of capabilities.components ?? []) option(control.components, policy, policy);
    control.candidates.max = String(capabilities.max_candidates ?? 1);
    for (const stage of ["simplify", "arjun"]) control[stage].checked = available(stage);
    // A setting this build does not take cannot be changed, and is not sent.
    for (const [name, key] of SETTING_KEYS) {
      if (!accepts(key)) {
        control[name].disabled = true;
        control[name].title = "This build of vitri does not take this setting.";
      }
    }
    if (addressSettings !== null) applySettings(addressSettings);
    addressSettings = null;
    updateStages();
  }

  // The stage boxes are fixed when this build lacks the stage, and Arjun is
  // fixed under compile, whose preprocessing has no Arjun stage.
  function updateStages() {
    const compile = control.mode.value === "compile";
    for (const stage of ["simplify", "arjun"]) {
      control[stage].disabled = !available(stage) || (stage === "arjun" && compile);
    }
    const reasons = [
      available("simplify") ? "" : "This build of vitri has no simplify stage.",
      !available("arjun") ? "This build of vitri does not include Arjun." : compile ? "Compilation has no Arjun stage." : "",
    ].filter(Boolean);
    const note = element("arjun-note");
    note.hidden = reasons.length === 0;
    note.textContent = reasons.join(" ");
  }

  function applySettings(params) {
    const get = (key) => params.get(key);
    const pick = (select, value) => {
      if (value !== null && [...select.options].some((item) => item.value === value)) select.value = value;
    };
    pick(control.mode, get("mode"));
    if (get("vtree") !== null && get("vtree").length <= 200) control.vtree.value = get("vtree");
    if (get("budget") !== null && /^\d{1,9}$/.test(get("budget"))) control.budget.value = get("budget");
    pick(control.components, get("components"));
    if (get("candidates") !== null && /^\d{1,3}$/.test(get("candidates"))) control.candidates.value = get("candidates");
    for (const stage of ["simplify", "arjun"]) {
      const value = get(stage);
      if (available(stage) && (value === "0" || value === "1")) control[stage].checked = value === "1";
    }
  }

  // The request for the worker: only what differs from vitri's defaults, so
  // the summary's echo of the effective settings stays the one record of
  // them. Returns {request} or {error}.
  function buildRequest() {
    const request = { format: capabilities.request_format ?? "vitri-request-v1" };
    if (control.mode.value !== "") request.mode = control.mode.value;
    const spec = control.vtree.value.trim();
    if (spec !== "" && spec !== capabilities.default_vtree) request.vtree = spec;
    const budget = control.budget.value.trim();
    if (budget !== "" || control.budget.validity.badInput) {
      const value = Number(budget);
      if (!Number.isSafeInteger(value) || value < 1) {
        return { error: "Budget is a whole number of milliseconds, 1 or more. Leave it empty for no budget." };
      }
      request.budget_ms = value;
    }
    if (control.components.value !== "") request.components = control.components.value;
    const most = capabilities.max_candidates ?? 1;
    const candidates = Number(control.candidates.value);
    if (!Number.isInteger(candidates) || candidates < 1 || candidates > most) {
      return { error: `Candidates is a whole number from 1 to ${most}.` };
    }
    if (candidates !== 1) request.candidates = candidates;
    for (const stage of ["simplify", "arjun"]) {
      if (!available(stage) || control[stage].checked) continue;
      // vitri refuses arjun under compile, where the stage does not exist.
      if (stage === "arjun" && request.mode === "compile") continue;
      request[stage] = false;
    }
    for (const key of Object.keys(request)) {
      if (key !== "format" && !accepts(key)) delete request[key];
    }
    return { request };
  }

  element("settings").addEventListener("input", markStale);
  control.mode.addEventListener("change", updateStages);
  element("settings").addEventListener("submit", (event) => {
    event.preventDefault();
    requestRun();
  });

  // The address carries the example and the settings, so a result can be
  // linked to. It is read once when the page opens and written at every run.
  // A formula typed or opened is not in the address, so a run on one keeps
  // only the settings there and hides the button that copies it.
  function writeAddress() {
    const params = new URLSearchParams();
    const example = chosenExample();
    if (example >= 0) params.set("example", EXAMPLES[example][0]);
    if (control.mode.value !== "") params.set("mode", control.mode.value);
    if (control.vtree.value.trim() !== "" && control.vtree.value.trim() !== capabilities.default_vtree) {
      params.set("vtree", control.vtree.value.trim());
    }
    if (control.budget.value.trim() !== "") params.set("budget", control.budget.value.trim());
    if (capabilities.components && control.components.value !== capabilities.components[0]) {
      params.set("components", control.components.value);
    }
    if (control.candidates.value !== "1") params.set("candidates", control.candidates.value);
    for (const stage of ["simplify", "arjun"]) {
      if (available(stage) && !control[stage].checked) params.set(stage, "0");
    }
    const query = params.toString();
    history.replaceState(null, "", query === "" ? location.pathname : `${location.pathname}?${query}`);
    element("link").hidden = example < 0;
  }

  function flash(button, text) {
    const label = button.textContent;
    button.textContent = text;
    setTimeout(() => {
      button.textContent = label;
    }, 1500);
  }
  element("link").addEventListener("click", () => {
    navigator.clipboard.writeText(location.href).then(
      () => flash(element("link"), "Copied"),
      () => flash(element("link"), "Not copied"),
    );
  });

  // --------------------------------------------------------------- worker

  // vitri runs in a worker, so the page stays live during a run, the status
  // can count the seconds, and a run can be stopped: Cancel ends the worker
  // and starts another, which loads the module again. A run asked for before
  // the worker is ready, or while another runs, starts once it is.
  let worker = null;
  let ready = false;
  let running = false;
  let runWhenReady = false;
  let ticking = 0;
  let sent = null;

  function startWorker() {
    ready = false;
    element("run").disabled = true;
    worker = new Worker("worker.js" + stamp);
    worker.onmessage = (event) => {
      const message = event.data;
      if (message.ready === true) {
        const first = capabilities === null;
        capabilities = message.capabilities;
        if (first) setUpControls();
        ready = true;
        element("run").disabled = false;
        if (!running) statusLine.textContent = "Ready.";
        if (runWhenReady) {
          runWhenReady = false;
          run();
        }
      } else if (message.error !== undefined) {
        if (running) {
          settle();
          showFailure(message.error);
        } else {
          statusLine.textContent = `vitri did not load: ${message.error.message}`;
        }
      } else if (message.result !== undefined && running) {
        finish(message.result, message.elapsed);
      }
    };
    worker.onerror = (event) => {
      event.preventDefault();
      const text = event.message || "the worker stopped";
      if (running) {
        settle();
        showFailure({ kind: "worker", message: text });
        restartWorker();
      } else {
        statusLine.textContent = `vitri did not load: ${text}`;
      }
    };
  }

  function restartWorker() {
    worker.terminate();
    startWorker();
  }

  function requestRun() {
    if (running) {
      settle();
      restartWorker();
      runWhenReady = true;
    } else if (ready) {
      run();
    } else {
      runWhenReady = true;
      statusLine.textContent = "Loading vitri; the run starts when it is ready.";
    }
  }

  function settle() {
    running = false;
    clearInterval(ticking);
    document.body.classList.remove("busy");
    element("cancel").hidden = true;
    element("run").hidden = false;
  }

  function run() {
    const text = inputText();
    if (text.trim() === "") {
      statusLine.textContent = "Paste, open or drop a DIMACS CNF first.";
      return;
    }
    const built = buildRequest();
    if (built.error !== undefined) {
      statusLine.textContent = built.error;
      return;
    }
    writeAddress();
    running = true;
    sent = { request: built.request, name: inputName, example: chosenExample() };
    const started = performance.now();
    statusLine.textContent = "Running.";
    ticking = setInterval(() => {
      statusLine.textContent = `Running, ${Math.round((performance.now() - started) / 1000)} s.`;
    }, 1000);
    element("run").hidden = true;
    element("cancel").hidden = false;
    document.body.classList.add("busy");
    worker.postMessage({ dimacs: text, request: built.request });
  }

  element("cancel").addEventListener("click", () => {
    if (!running) return;
    settle();
    restartWorker();
    statusLine.textContent = "Stopped. Loading vitri again.";
  });

  function markStale() {
    if (current !== null) element("stale").hidden = false;
  }

  // ---------------------------------------------------------------- result

  const parseJson = (text) => {
    if (typeof text !== "string") return null;
    try {
      return JSON.parse(text);
    } catch {
      return null;
    }
  };

  function finish(result, elapsed) {
    settle();
    element("stale").hidden = true;
    const summary = result === null || typeof result !== "object" ? null : result.summary;
    if (summary === null || typeof summary !== "object" || summary.format !== "vitri-result-v1") {
      showFailure({ kind: "format", message: `the result format ${JSON.stringify(summary?.format)} is not one this page reads` });
      return;
    }
    const files = result.files ?? {};
    const record = parseJson(files["preprocess.json"]);
    let manifest = parseJson(files["components.json"]);
    let manifestNote = "";
    if (manifest !== null && !/^vitri-components-v[12]$/.test(manifest.format)) {
      manifestNote = `components.json has format ${JSON.stringify(manifest.format)}, which this page does not read.`;
      manifest = null;
    }
    current = { summary, files, elapsed, record, manifest, manifestNote, sent, trees: new Map(), scenes: new Map(), counts: new Map() };
    statusLine.textContent = `Done in ${duration(elapsed)}.`;
    renderSummary();
    renderExports();
    element("panels").hidden = false;
    fillFileChoice();
    fillTreeChoice();
  }

  function showFailure(error) {
    element("stale").hidden = true;
    current = null;
    scene = null;
    const kind = error && typeof error.kind === "string" ? error.kind : "worker";
    const message = error && typeof error.message === "string" ? error.message : String(error);
    statusLine.textContent = "Not prepared; see the result.";
    const box = element("summary");
    const line = make("div", "status-line failed");
    const heading = make("p");
    heading.style.margin = "0";
    heading.append(make("strong", "", "Not prepared: "), document.createTextNode(`${ERROR_KINDS[kind] ?? kind}.`));
    line.append(heading, make("pre", "", message));
    box.replaceChildren(line);
    element("exports").hidden = true;
    element("panels").hidden = true;
  }

  function renderSummary() {
    const { summary, manifest, manifestNote } = current;
    const box = element("summary");
    const status = summary.status;
    const line = make("div", `status-line ${status}`);
    const p = make("p");
    p.style.margin = "0";
    if (status === "built") {
      const variables = summary.reduced?.variables ?? 0;
      p.append(make("strong", "", "Built. "), document.createTextNode(
        `Preprocessing left ${plural(variables, "variable")} to compile, and vitri built a vtree over ${variables === 1 ? "it" : "them"}.`,
      ));
    } else if (status === "fully_resolved") {
      p.append(make("strong", "", "Fully resolved. "), document.createTextNode(
        "Preprocessing settled every variable, so there is nothing to compile and no vtree. " +
          "The count of the original formula is the lift below, and the bundle is reduced.cnf and preprocess.json alone.",
      ));
    } else if (status === "refuted") {
      p.append(make("strong", "", "Refuted. "), document.createTextNode(
        "Preprocessing proved the formula unsatisfiable, so its count is 0 and there is nothing to compile. " +
          "reduced.cnf holds an explicit contradiction (x and not x), because DIMACS has no portable empty clause; " +
          "the bundle is reduced.cnf and preprocess.json alone.",
      ));
    } else {
      p.append(make("strong", "", `Status ${status}.`));
    }
    line.append(p);

    const stats = make("div", "stats");
    const stat = (value, word, from) => {
      const item = make("span", "stat");
      const figure = make("b", "", value);
      item.append(figure, document.createTextNode(word));
      if (from !== undefined) item.append(make("span", "from", from));
      stats.append(item);
    };
    const count = (n) => (typeof n === "number" ? n.toLocaleString("en-US") : "?");
    stat(count(summary.reduced?.variables), "variables", `of ${count(summary.input?.variables)}`);
    stat(count(summary.reduced?.clauses), "clauses", `of ${count(summary.input?.clauses)}`);
    if (summary.vtree) {
      stat(count(summary.vtree.components), summary.vtree.components === 1 ? "component" : "components");
      stat(count(summary.vtree.free_variables), summary.vtree.free_variables === 1 ? "free variable" : "free variables");
    }
    stat(duration(current.elapsed), "");

    const facts = make("dl", "facts");
    const fact = (term, ...values) => {
      facts.append(make("dt", "", term));
      const dd = make("dd");
      for (const value of values) dd.append(typeof value === "string" ? document.createTextNode(value) : value);
      facts.append(dd);
    };
    const mode = summary.mode;
    const detected = current.sent !== null && current.sent.request.mode === undefined;
    fact("Mode", `${mode}${MODE_NAMES[mode] ? `, ${MODE_NAMES[mode]}` : ""}${detected ? " (detected from the file)" : ""}`);

    const lift = summary.lift ?? {};
    const liftBox = make("span");
    if (status === "refuted") {
      liftBox.append(make("span", "lift", "count(original) = 0"));
    } else if (status === "fully_resolved") {
      liftBox.append(make("span", "lift", `count(original) = ${lift.factor}`));
    } else {
      liftBox.append(make("span", "lift", `count(original) = count(reduced) × ${lift.factor}`));
      const notes = ["count(reduced) comes from your compiler or model counter; vitri does not count."];
      if (typeof mode === "string" && mode.includes("w") && mode !== "compile") {
        notes.push("Take it under the weights written in reduced.cnf, not the input's.");
      }
      if (typeof mode === "string" && mode.startsWith("p")) {
        notes.push("Take it over the show set written in reduced.cnf, not the input's.");
      }
      liftBox.append(make("br"), make("span", "note", notes.join(" ")));
    }
    fact("Lift", liftBox);
    if (lift.count_lift_pow2 !== undefined) {
      fact("Recorded as", `count_lift_pow2 ${lift.count_lift_pow2}, weight_lift ${lift.weight_lift}`);
    }

    // A null outcome is a stage this run's chain did not reach or does not
    // have in this mode; only the others are listed.
    const reached = Object.entries(summary.stages ?? {}).filter(([, outcome]) => outcome !== null);
    fact("Stages", reached.length === 0
      ? "none ran"
      : reached.map(([stage, outcome]) => `${STAGE_NAMES[stage] ?? stage}: ${OUTCOMES[outcome] ?? outcome}`).join("; "));

    if (summary.vtree && manifest !== null) {
      const winners = new Map();
      for (const entry of manifest.components ?? []) {
        const spec = entry.selection?.winning_spec;
        if (spec) winners.set(spec, (winners.get(spec) ?? 0) + 1);
      }
      const chosen = [...winners].map(([spec, times]) => (manifest.components.length > 1 ? `${spec} (${times})` : spec));
      fact("Construction", `${summary.request?.vtree ?? "?"}${chosen.length ? `, selected ${chosen.join(", ")}` : ""}`);
      fact("Vtree", `${plural(summary.vtree.leaves, "leaf")}, ${plural(summary.vtree.nodes, "node")}`);
    }
    if (manifestNote !== "") fact("Components", manifestNote);
    const request = summary.request ?? {};
    const settings = [
      `budget ${request.budget_ms === null || request.budget_ms === undefined ? "unbounded" : `${request.budget_ms} ms`}`,
      `components ${request.components}`,
      `candidates ${request.candidates}`,
      `simplify ${request.simplify ? "on" : "off"}`,
    ];
    // A build without Arjun cannot run it whatever the request says.
    if (available("arjun")) settings.push(`Arjun ${request.arjun ? "on" : "off"}`);
    fact("Settings used", settings.join(", "));
    fact("vitri", String(summary.vitri_version ?? "?"));
    box.replaceChildren(line, stats, facts);
  }

  // ---------------------------------------------------------------- saving

  function saveBlob(blob, name) {
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = name;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(link.href), 5000);
  }

  // The stem the saved files are named after: the file that was opened, the
  // example, or "formula".
  function stem() {
    const sentBy = current.sent;
    if (sentBy !== null && sentBy.name) return sentBy.name.replace(/\.(cnf|dimacs|txt)(\.gz)?$/i, "").replace(/[^\w.-]+/g, "_") || "formula";
    if (sentBy !== null && sentBy.example >= 0) return EXAMPLES[sentBy.example][0];
    return "formula";
  }

  function bundlePaths() {
    const order = ["reduced.cnf", "preprocess.json", "vtree.vtree", "components.json"];
    const paths = Object.keys(current.files);
    return [...order.filter((path) => paths.includes(path)), ...paths.filter((path) => !order.includes(path)).sort()];
  }

  function renderExports() {
    const exports = element("exports");
    exports.hidden = false;
    for (const button of exports.querySelectorAll("button")) {
      const what = button.dataset.save;
      if (what === "zip") button.disabled = false;
      else if (what === "svg" || what === "png") button.disabled = current.summary.status !== "built";
      else button.disabled = typeof current.files[what] !== "string";
    }
  }

  element("exports").addEventListener("click", async (event) => {
    const button = event.target.closest("button");
    if (button === null || current === null) return;
    const what = button.dataset.save;
    try {
      if (what === "zip") {
        const base = stem();
        const zip = zipStore(bundlePaths().map((path) => ({ name: `${base}/${path}`, data: current.files[path] })));
        saveBlob(new Blob([zip], { type: "application/zip" }), `${base}.zip`);
      } else if (what === "svg") {
        if (scene === null) return;
        const text = sceneSvg(scene, readColors(), `${scene.view.path}`);
        saveBlob(new Blob([text], { type: "image/svg+xml" }), `${stem()}-${scene.view.fileStem}.svg`);
      } else if (what === "png") {
        if (scene === null) return;
        const blob = await scenePng(scene);
        saveBlob(blob, `${stem()}-${scene.view.fileStem}.png`);
      } else if (typeof current.files[what] === "string") {
        saveBlob(new Blob([current.files[what]], { type: "text/plain" }), what);
      }
    } catch (failure) {
      flash(button, "Not saved");
      statusLine.textContent = `Not saved: ${failure.message ?? failure}`;
    }
  });

  function scenePng(shown) {
    const pad = 20;
    const world = { w: shown.layout.width + 2 * pad, h: shown.layout.height + 2 * pad };
    const scale = Math.min(2, PNG_SIDE / world.w, PNG_SIDE / world.h, Math.sqrt(PNG_AREA / (world.w * world.h)));
    const image = document.createElement("canvas");
    image.width = Math.max(1, Math.floor(world.w * scale));
    image.height = Math.max(1, Math.floor(world.h * scale));
    const context = image.getContext("2d");
    const colors = readColors();
    context.fillStyle = colors.card;
    context.fillRect(0, 0, image.width, image.height);
    context.setTransform(scale, 0, 0, scale, pad * scale, (pad + LEAF_HEIGHT / 2) * scale);
    drawScene(context, shown, colors, { x0: -Infinity, y0: -Infinity, x1: Infinity, y1: Infinity }, scale);
    return new Promise((resolve, reject) => {
      image.toBlob((blob) => (blob === null ? reject(new Error("the browser did not make the image")) : resolve(blob)), "image/png");
    });
  }

  // ------------------------------------------------------------ file view

  const text = { content: "", starts: new Uint32Array(1), lines: 0, rows: 0 };
  const LINE = 20;

  function fillFileChoice() {
    const select = element("file-choice");
    select.replaceChildren();
    for (const path of bundlePaths()) {
      const item = make("option", "", path);
      item.value = path;
      select.append(item);
    }
    select.value = "reduced.cnf";
    showFile(select.value);
  }

  function showFile(path) {
    const content = current.files[path] ?? "";
    let lines = 0;
    for (let at = content.indexOf("\n"); at !== -1; at = content.indexOf("\n", at + 1)) lines += 1;
    if (content.length > 0 && !content.endsWith("\n")) lines += 1;
    const starts = new Uint32Array(lines + 1);
    let widest = 0;
    let line = 0;
    let begin = 0;
    for (let at = content.indexOf("\n"); at !== -1 && line < lines; at = content.indexOf("\n", at + 1)) {
      starts[line++] = begin;
      widest = Math.max(widest, at - begin);
      begin = at + 1;
    }
    if (line < lines) {
      starts[line++] = begin;
      widest = Math.max(widest, content.length - begin);
    }
    starts[lines] = content.length + 1;
    Object.assign(text, { content, starts, lines });
    const spacer = element("text-spacer");
    spacer.style.height = `${Math.min(lines * LINE, MAX_SCROLL_HEIGHT)}px`;
    spacer.style.width = `calc(${Math.min(widest, SHOWN_LINE_CHARS) + 1}ch + 6.5em)`;
    textView.scrollTop = 0;
    textView.scrollLeft = 0;
    element("file-info").textContent = `${plural(lines, "line")}, ${megabytes(new Blob([content]).size)}`;
    renderLines();
  }

  let linesQueued = false;
  function renderLines() {
    linesQueued = false;
    const inView = Math.ceil(textView.clientHeight / LINE) + 1;
    const total = text.lines * LINE;
    const scaled = total > MAX_SCROLL_HEIGHT;
    const room = Math.max(1, Math.min(total, MAX_SCROLL_HEIGHT) - textView.clientHeight);
    const first = scaled
      ? Math.min(Math.max(0, text.lines - inView + 1), Math.round((textView.scrollTop / room) * Math.max(0, text.lines - inView + 1)))
      : Math.floor(textView.scrollTop / LINE);
    const holder = element("text-lines");
    holder.style.transform = `translateY(${scaled ? textView.scrollTop : first * LINE}px)`;
    const rows = [];
    for (let i = first; i < Math.min(text.lines, first + inView); i++) {
      const begin = text.starts[i];
      const end = text.starts[i + 1] - 1;
      const row = make("div");
      row.append(make("span", "", String(i + 1)));
      if (end - begin > SHOWN_LINE_CHARS) {
        row.append(document.createTextNode(text.content.slice(begin, begin + SHOWN_LINE_CHARS)));
        row.append(make("em", "", ` … ${(end - begin - SHOWN_LINE_CHARS).toLocaleString("en-US")} more characters; Save has the whole line`));
      } else {
        row.append(document.createTextNode(text.content.slice(begin, end).replace(/\r$/, "")));
      }
      rows.push(row);
    }
    if (text.lines === 0) rows.push(make("div", "cut", "(empty file)"));
    holder.replaceChildren(...rows);
  }
  textView.addEventListener("scroll", () => {
    if (!linesQueued) {
      linesQueued = true;
      requestAnimationFrame(renderLines);
    }
  });
  new ResizeObserver(() => {
    if (current !== null) renderLines();
  }).observe(textView);
  element("file-choice").addEventListener("change", (event) => showFile(event.target.value));

  // ------------------------------------------------------------ tree views

  // The vtrees a result offers: the whole formula's, each component's own
  // when there are several, and the retained candidates.
  function treeViews() {
    const { summary, files, manifest } = current;
    const views = [];
    if (summary.status !== "built" || typeof files["vtree.vtree"] !== "string") return views;
    views.push({ key: "whole", path: "vtree.vtree", label: "whole formula (vtree.vtree)", fileStem: "vtree", component: -1, cnf: "reduced.cnf" });
    const entries = manifest === null ? [] : manifest.components ?? [];
    entries.forEach((entry, i) => {
      const name = `component ${String(i).padStart(3, "0")}`;
      if (entry.vtree !== "vtree.vtree" && typeof files[entry.vtree] === "string") {
        const spec = entry.selection?.winning_spec ? `, ${entry.selection.winning_spec}` : "";
        views.push({
          key: `c${i}`, path: entry.vtree, component: i, cnf: entry.cnf,
          label: `${name}: ${plural(entry.local_to_reduced_dimacs.length, "variable")}${spec}`,
          fileStem: entry.vtree.replace(/^.*\//, "").replace(/\.vtree$/, ""),
        });
      }
      (entry.vtree_candidates ?? []).forEach((candidate, rank) => {
        if (rank === 0 || typeof files[candidate.vtree] !== "string") return;
        views.push({
          key: `c${i}r${rank}`, path: candidate.vtree, component: i, cnf: entry.cnf, builtBy: candidate.built_by,
          label: `${entries.length > 1 ? `${name} ` : ""}candidate ${rank}: ${(candidate.built_by ?? []).join(", ")}`,
          fileStem: candidate.vtree.replace(/^.*\//, "").replace(/\.vtree$/, ""),
        });
      });
    });
    return views;
  }

  function fillTreeChoice() {
    const select = element("tree-choice");
    select.replaceChildren();
    const views = treeViews();
    current.views = views;
    for (const view of views) {
      const item = make("option", "", view.label);
      item.value = view.key;
      select.append(item);
    }
    select.disabled = views.length <= 1;
    const empty = element("tree-empty");
    const zoomButtons = element("zoom").querySelectorAll("button");
    if (views.length === 0) {
      scene = null;
      empty.hidden = false;
      empty.textContent = current.summary.status === "refuted"
        ? "No vtree: preprocessing refuted the formula."
        : current.summary.status === "fully_resolved"
          ? "No vtree: preprocessing resolved every variable."
          : "No vtree in this result.";
      zoomButtons.forEach((button) => (button.disabled = true));
      element("details").hidden = true;
      element("tree-description").textContent = empty.textContent;
      element("outline").replaceChildren();
      clearCanvas();
      return;
    }
    zoomButtons.forEach((button) => (button.disabled = false));
    element("details").hidden = false;
    showTree(views[0]);
  }

  element("tree-choice").addEventListener("change", (event) => {
    const view = current.views.find((item) => item.key === event.target.value);
    if (view === undefined) return;
    showTree(view);
    if (typeof current.files[view.cnf] === "string") {
      element("file-choice").value = view.cnf;
      showFile(view.cnf);
    }
  });

  // Component of every node of the whole-formula tree: a leaf takes its
  // variable's component, or -2 for a free variable; an internal node takes
  // its children's when they agree and -1 when they do not, which marks the
  // nodes that join components.
  function componentsOf(tree) {
    const { manifest } = current;
    if (manifest === null) return null;
    const entries = manifest.components ?? [];
    const free = manifest.free_vars_reduced_dimacs ?? [];
    if (entries.length <= 1 && free.length === 0) return null;
    const of = new Map();
    entries.forEach((entry, i) => entry.local_to_reduced_dimacs.forEach((reduced) => of.set(reduced, i)));
    for (const reduced of free) of.set(reduced, -2);
    const component = new Int32Array(tree.count);
    for (let i = tree.count - 1; i >= 0; i--) {
      const node = tree.order[i];
      if (isLeaf(tree, node)) component[node] = of.get(tree.variable[node]) ?? -1;
      else {
        const a = component[tree.left[node]];
        component[node] = a >= 0 && a === component[tree.right[node]] ? a : -1;
      }
    }
    return component;
  }

  function showTree(view) {
    element("tree-empty").hidden = true;
    let shown = current.scenes.get(view.key);
    if (shown === undefined) {
      const tree = parseVtree(current.files[view.path]);
      if (tree.error !== undefined) {
        scene = null;
        const empty = element("tree-empty");
        empty.hidden = false;
        empty.textContent = `${view.path} was not read: ${tree.error}.`;
        element("tree-description").textContent = empty.textContent;
        clearCanvas();
        return;
      }
      const sizes = sizesFor(tree);
      const room = Math.max(300, treeView.clientWidth);
      const budget = Math.min(INITIAL_NODES, Math.max(63, 2 * Math.floor(room / (sizes.slot * 0.6))));
      shown = {
        view, tree, sizes,
        expanded: initialExpansion(tree, budget),
        budget,
        x: new Float64Array(tree.count),
        component: view.component === -1 ? componentsOf(tree) : null,
        selected: -1,
        onPath: new Uint8Array(tree.count),
        scale: 1, tx: 0, ty: 0,
        fitted: false,
      };
      shown.layout = layoutTree(tree, shown.expanded, sizes, shown.x);
      current.scenes.set(view.key, shown);
    }
    scene = shown;
    if (!scene.fitted) {
      fit();
      scene.fitted = true;
    }
    element("tree-choice").value = view.key;
    describeTree();
    renderDetails();
    draw();
  }

  // --------------------------------------------------------- tree drawing

  let colorCache = null;
  function readColors() {
    if (colorCache !== null) return colorCache;
    const style = getComputedStyle(document.documentElement);
    const token = (name) => style.getPropertyValue(name).trim();
    colorCache = {
      card: token("--card"),
      code: token("--code"),
      ink: token("--ink"),
      muted: token("--muted"),
      accent: token("--accent"),
      edge: token("--edge"),
      pick: token("--pick"),
      mono: token("--mono"),
      components: [1, 2, 3, 4, 5, 6, 7, 8].map((i) => token(`--comp-${i}`)),
    };
    return colorCache;
  }
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    colorCache = null;
    draw();
  });

  function clearCanvas() {
    const context = canvas.getContext("2d");
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, canvas.width, canvas.height);
  }

  let drawQueued = false;
  function scheduleDraw() {
    if (drawQueued) return;
    drawQueued = true;
    requestAnimationFrame(() => {
      drawQueued = false;
      draw();
    });
  }

  function draw() {
    if (scene === null) return;
    const ratio = window.devicePixelRatio || 1;
    const width = treeView.clientWidth;
    const height = treeView.clientHeight;
    if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
      canvas.width = Math.round(width * ratio);
      canvas.height = Math.round(height * ratio);
    }
    const context = canvas.getContext("2d");
    const colors = readColors();
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.fillStyle = colors.code;
    context.fillRect(0, 0, width, height);
    const { scale, tx, ty } = scene;
    context.setTransform(ratio * scale, 0, 0, ratio * scale, ratio * tx, ratio * ty);
    const bounds = { x0: -tx / scale, y0: -ty / scale, x1: (width - tx) / scale, y1: (height - ty) / scale };
    drawScene(context, scene, colors, bounds, scale);
    element("zoom-level").textContent = `${Math.round(scale * 100)}%`;
  }

  const MIN_SCALE = 0.0005;
  const MAX_SCALE = 6;

  function fit() {
    const width = treeView.clientWidth || 600;
    const height = treeView.clientHeight || 400;
    const pad = 24;
    const { layout } = scene;
    const scale = Math.min(1.5, (width - 2 * pad) / Math.max(1, layout.width), (height - 2 * pad) / Math.max(1, layout.height));
    scene.scale = Math.max(MIN_SCALE, scale);
    scene.tx = (width - layout.width * scene.scale) / 2;
    scene.ty = (height - layout.height * scene.scale) / 2 + (LEAF_HEIGHT / 2) * scene.scale;
  }

  function zoomAt(factor, sx, sy) {
    const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, scene.scale * factor));
    const wx = (sx - scene.tx) / scene.scale;
    const wy = (sy - scene.ty) / scene.scale;
    scene.scale = scale;
    scene.tx = sx - wx * scale;
    scene.ty = sy - wy * scale;
    scheduleDraw();
  }

  // Re-lays the tree after an expansion changed, keeping `anchor` where it
  // was on screen.
  function relayout(anchor) {
    const before = anchor >= 0 ? scene.x[anchor] * scene.scale + scene.tx : 0;
    scene.layout = layoutTree(scene.tree, scene.expanded, scene.sizes, scene.x);
    if (anchor >= 0) scene.tx = before - scene.x[anchor] * scene.scale;
    describeTree();
    scheduleDraw();
  }

  function toggle(node) {
    const { tree, expanded } = scene;
    if (node < 0 || isLeaf(tree, node)) return;
    if (expanded[node] === 1) {
      expanded[node] = 0;
      if (scene.selected >= 0 && scene.selected !== node && isUnder(tree, scene.selected, node)) select(node, false);
      announce(`Collapsed node ${node}; ${plural(tree.leaves[node], "leaf")} below it.`);
    } else {
      expanded[node] = 1;
      announce(`Expanded node ${node}.`);
    }
    relayout(node);
    renderDetails();
  }

  function expandSelected() {
    const node = scene.selected >= 0 ? scene.selected : scene.tree.root;
    const { added, stopped } = expandBelow(scene.tree, scene.expanded, node, EXPAND_NODES);
    relayout(node);
    announce(stopped
      ? `Expanded ${plural(added, "node")} below node ${node}; deeper subtrees are still collapsed.`
      : `Expanded everything below node ${node}, ${plural(added, "node")}.`);
    renderDetails();
  }

  function resetView() {
    scene.expanded = initialExpansion(scene.tree, scene.budget);
    scene.layout = layoutTree(scene.tree, scene.expanded, scene.sizes, scene.x);
    let node = scene.selected;
    while (node >= 0 && !isDrawn(node)) node = scene.tree.parent[node];
    if (scene.selected >= 0) select(node, false);
    fit();
    describeTree();
    renderDetails();
    scheduleDraw();
  }

  // Whether a node is drawn: every node above it is expanded.
  function isDrawn(node) {
    for (let up = scene.tree.parent[node]; up >= 0; up = scene.tree.parent[up]) {
      if (scene.expanded[up] === 0) return false;
    }
    return true;
  }

  function select(node, keepInView = true) {
    const { tree, onPath } = scene;
    for (let at = scene.selected; at >= 0; at = tree.parent[at]) onPath[at] = 0;
    scene.selected = node;
    for (let at = node; at >= 0; at = tree.parent[at]) onPath[at] = 1;
    if (node >= 0 && keepInView) {
      const width = treeView.clientWidth;
      const height = treeView.clientHeight;
      const sx = scene.x[node] * scene.scale + scene.tx;
      const sy = tree.depth[node] * ROW * scene.scale + scene.ty;
      if (sx < 40 || sx > width - 40) scene.tx += width / 2 - sx;
      if (sy < 40 || sy > height - 60) scene.ty += height / 2 - sy;
    }
    renderDetails();
    if (node >= 0) announce(nodeSentence(node));
    scheduleDraw();
  }

  const announce = (sentence) => {
    element("tree-live").textContent = sentence;
  };

  function nodeSentence(node) {
    const { tree } = scene;
    if (isLeaf(tree, node)) {
      const chain = variableChain(tree.variable[node]);
      return `Leaf node ${node} at depth ${tree.depth[node]}: ${chain.map(([space, value]) => `${space} ${value}`).join(", ")}.`;
    }
    const state = scene.expanded[node] === 1 ? "expanded" : "collapsed";
    return `Internal node ${node} at depth ${tree.depth[node]}, ${state}, ${plural(tree.leaves[node], "leaf")} below.`;
  }

  // What a leaf's variable is in each numbering: [space, text] pairs from the
  // tree's own space out to the original formula.
  function variableChain(variable) {
    const { view } = scene;
    const { record, manifest } = current;
    if (view.component === -1) {
      return [["reduced variable", String(variable)], ["original", originalLiteral(record, variable)]];
    }
    const entry = manifest.components[view.component];
    const map = entry.local_to_reduced_dimacs;
    if (variable < 1 || variable > map.length) {
      return [["local variable", String(variable)], ["reduced", "outside local_to_reduced_dimacs"]];
    }
    const reduced = map[variable - 1];
    return [["local variable", String(variable)], ["reduced variable", String(reduced)], ["original", originalLiteral(record, reduced)]];
  }

  function occurrenceCounts() {
    const { view, tree } = scene;
    const key = view.cnf;
    if (!current.counts.has(key)) {
      const content = current.files[key];
      const header = typeof content === "string" ? dimacsHeader(content) : null;
      current.counts.set(key, header === null ? null : occurrences(content, Math.max(header.variables, tree.maxVariable)));
    }
    return current.counts.get(key);
  }

  function renderDetails() {
    const list = element("details-list");
    const rows = [];
    const row = (term, value, className) => {
      rows.push(make("dt", "", term));
      const dd = make("dd", className);
      if (typeof value === "string") dd.textContent = value;
      else dd.append(value);
      rows.push(dd);
    };
    if (scene === null || scene.selected < 0) {
      row("", "Click a node, or focus the drawing and press an arrow key.");
      list.replaceChildren(...rows);
      return;
    }
    const { tree, selected: node, view, component } = scene;
    const { manifest } = current;
    row("Node", `${node} in ${view.path}${tree.parent[node] < 0 ? ", the root" : ""}`);
    row("Depth", String(tree.depth[node]));
    if (isLeaf(tree, node)) {
      const variable = tree.variable[node];
      const chain = variableChain(variable);
      for (const [space, value] of chain) row(space[0].toUpperCase() + space.slice(1), value);
      const tokens = [];
      if (view.component !== -1) tokens.push(`local ${variable}`, `reduced ${chain[1][1]}`);
      else tokens.push(`reduced ${variable}`);
      const reduced = view.component === -1 ? variable : Number(chain[1][1]);
      if (Number.isInteger(reduced)) tokens.push(`original ${originalToken(current.record, reduced)}`);
      row("Numbering", tokens.join(" → "), "chain");
      if (view.component === -1 && manifest !== null) {
        const free = (manifest.free_vars_reduced_dimacs ?? []).includes(variable);
        const index = (manifest.components ?? []).findIndex((entry) => entry.local_to_reduced_dimacs.includes(variable));
        if (free) row("Component", "none: a free variable, in no clause of reduced.cnf");
        else if (index >= 0) {
          const local = manifest.components[index].local_to_reduced_dimacs.indexOf(variable) + 1;
          const swatch = make("span");
          if (component !== null) {
            const chip = make("span", "swatch");
            chip.style.background = readColors().components[index % 8];
            swatch.append(chip);
          }
          swatch.append(document.createTextNode(`${String(index).padStart(3, "0")}, as local variable ${local}`));
          row("Component", swatch);
        }
      }
      const counts = occurrenceCounts();
      if (counts !== null && variable < counts.length) row("Clauses", `occurs in ${plural(counts[variable], "clause")} of ${view.cnf}`);
    } else {
      row("Leaves below", tree.leaves[node].toLocaleString("en-US"));
      const { variables, all } = leafVariables(tree, node, LISTED_LEAVES);
      row(view.component === -1 ? "Variables" : "Local variables", `${variables.join(" ")}${all ? "" : " …"}`);
      row("Shown", scene.expanded[node] === 1 ? "expanded" : "collapsed; press Enter or double-click to expand");
      if (component !== null) {
        if (component[node] >= 0) row("Component", String(component[node]).padStart(3, "0"));
        else {
          const seen = new Set();
          let free = 0;
          for (let i = tree.pre[node]; i <= tree.last[node]; i++) {
            const below = tree.order[i];
            if (!isLeaf(tree, below)) continue;
            if (component[below] >= 0) seen.add(component[below]);
            else if (component[below] === -2) free += 1;
          }
          row("Joins", `${plural(seen.size, "component")}${free > 0 ? ` and ${plural(free, "free variable")}` : ""}`);
        }
      }
    }
    if (view.builtBy) row("Built by", view.builtBy.join(", "));
    list.replaceChildren(...rows);
  }

  function describeTree() {
    const { tree, layout, view, component } = scene;
    const parts = [
      `Showing the vtree of the ${view.label}: ${plural(tree.count, "node")}, ` +
        `${plural(tree.leaves[tree.root], "leaf")}, depth ${tree.height}.`,
      layout.collapsed > 0
        ? `${plural(layout.drawn, "node")} drawn; ${plural(layout.collapsed, "subtree")} collapsed, ` +
          (layout.collapsed === 1 ? "drawn as a triangle with its leaf count." : "drawn as triangles with their leaf counts.")
        : "Every node is drawn.",
    ];
    if (component !== null) {
      parts.push("Colours mark the components of reduced.cnf; grey internal nodes join components, and dashed leaves are free variables.");
    }
    element("tree-description").textContent = parts.join(" ");
    treeView.setAttribute("aria-label", `Vtree drawing, ${view.label}`);
    if (element("outline-box").open) renderOutline();
  }

  // The drawn part of the tree as nested lists, for reading without the
  // picture.
  function renderOutline() {
    const holder = element("outline");
    if (scene === null) {
      holder.replaceChildren();
      return;
    }
    const { tree, expanded } = scene;
    const root = make("ul");
    const stack = [[tree.root, root]];
    let items = 0;
    while (stack.length > 0 && items < OUTLINE_ITEMS) {
      const [node, list] = stack.pop();
      const item = make("li");
      if (isLeaf(tree, node)) {
        item.textContent = `leaf ${node}: variable ${tree.variable[node]}`;
      } else {
        const leaves = plural(tree.leaves[node], "leaf");
        item.textContent = `node ${node}: ${leaves}${expanded[node] === 1 ? "" : ", collapsed"}`;
        if (expanded[node] === 1) {
          const children = make("ul");
          item.append(children);
          stack.push([tree.right[node], children], [tree.left[node], children]);
        }
      }
      list.append(item);
      items += 1;
    }
    const parts = [root];
    if (stack.length > 0) parts.push(make("p", "note", `The list stops after ${OUTLINE_ITEMS.toLocaleString("en-US")} items.`));
    holder.replaceChildren(...parts);
  }
  element("outline-box").addEventListener("toggle", () => {
    if (element("outline-box").open) renderOutline();
  });

  // -------------------------------------------------------- tree controls

  element("zoom").addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (button === null || scene === null) return;
    const width = treeView.clientWidth;
    const height = treeView.clientHeight;
    const asked = button.dataset.zoom;
    if (asked === "in") zoomAt(1.25, width / 2, height / 2);
    else if (asked === "out") zoomAt(0.8, width / 2, height / 2);
    else if (asked === "fit") {
      fit();
      scheduleDraw();
    } else if (asked === "collapse") resetView();
    else if (asked === "expand") expandSelected();
  });

  treeView.addEventListener("wheel", (event) => {
    if (scene === null) return;
    event.preventDefault();
    const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? treeView.clientHeight : 1;
    const box = treeView.getBoundingClientRect();
    zoomAt(Math.exp(-event.deltaY * unit * 0.0015), event.clientX - box.left, event.clientY - box.top);
  }, { passive: false });

  let press = null;
  treeView.addEventListener("pointerdown", (event) => {
    if (scene === null || event.button !== 0) return;
    treeView.setPointerCapture(event.pointerId);
    press = { id: event.pointerId, x: event.clientX, y: event.clientY, tx: scene.tx, ty: scene.ty, moved: false };
  });
  treeView.addEventListener("pointermove", (event) => {
    if (press === null || event.pointerId !== press.id) return;
    const dx = event.clientX - press.x;
    const dy = event.clientY - press.y;
    if (!press.moved && Math.hypot(dx, dy) < 4) return;
    press.moved = true;
    treeView.classList.add("dragging");
    scene.tx = press.tx + dx;
    scene.ty = press.ty + dy;
    scheduleDraw();
  });
  const release = (event) => {
    if (press === null || event.pointerId !== press.id) return;
    const clicked = !press.moved && event.type === "pointerup";
    press = null;
    treeView.classList.remove("dragging");
    if (clicked) {
      const node = nodeUnder(event);
      if (node >= 0) select(node, false);
    }
  };
  treeView.addEventListener("pointerup", release);
  treeView.addEventListener("pointercancel", release);
  treeView.addEventListener("dblclick", (event) => {
    if (scene === null) return;
    const node = nodeUnder(event);
    if (node >= 0) toggle(node);
  });

  function nodeUnder(event) {
    const box = treeView.getBoundingClientRect();
    const wx = (event.clientX - box.left - scene.tx) / scene.scale;
    const wy = (event.clientY - box.top - scene.ty) / scene.scale;
    return nodeAt(scene.tree, scene.layout, scene.x, wx, wy, 4 / scene.scale);
  }

  // The drawn neighbour of a node in its row, one step left or right.
  function rowNeighbour(node, step) {
    const { layout, tree } = scene;
    const depth = tree.depth[node];
    for (let i = layout.rowStart[depth]; i < layout.rowStart[depth + 1]; i++) {
      if (layout.rows[i] !== node) continue;
      const j = i + step;
      return j >= layout.rowStart[depth] && j < layout.rowStart[depth + 1] ? layout.rows[j] : -1;
    }
    return -1;
  }

  treeView.addEventListener("keydown", (event) => {
    if (scene === null || event.altKey || event.ctrlKey || event.metaKey) return;
    const { tree, expanded } = scene;
    const node = scene.selected;
    const width = treeView.clientWidth;
    const height = treeView.clientHeight;
    const move = (target) => {
      if (target >= 0) select(target);
    };
    switch (event.key) {
      case "ArrowUp":
        move(node < 0 ? tree.root : tree.parent[node]);
        break;
      case "ArrowDown":
        if (node < 0) move(tree.root);
        else if (!isLeaf(tree, node)) {
          if (expanded[node] === 0) toggle(node);
          move(tree.left[node]);
        }
        break;
      case "ArrowLeft":
      case "ArrowRight": {
        if (node < 0) {
          move(tree.root);
          break;
        }
        const up = tree.parent[node];
        const sibling = up < 0 ? -1 : event.key === "ArrowLeft" ? tree.left[up] : tree.right[up];
        move(sibling >= 0 && sibling !== node ? sibling : rowNeighbour(node, event.key === "ArrowLeft" ? -1 : 1));
        break;
      }
      case "Home":
        move(tree.root);
        break;
      case "Enter":
      case " ":
        if (node >= 0) toggle(node);
        break;
      case "*":
        expandSelected();
        break;
      case "+":
      case "=":
        zoomAt(1.25, width / 2, height / 2);
        break;
      case "-":
      case "_":
        zoomAt(0.8, width / 2, height / 2);
        break;
      case "0":
        fit();
        scheduleDraw();
        break;
      default:
        return;
    }
    event.preventDefault();
  });

  new ResizeObserver(() => scheduleDraw()).observe(treeView);

  // ---------------------------------------------------------------- start

  startWorker();
  const params = new URLSearchParams(location.search);
  addressSettings = params;
  const opening = EXAMPLES.findIndex(([key]) => key === params.get("example"));
  if (cnfBox.value === "") loadExample(opening === -1 ? 0 : opening);
  else describeInput();
}

if (typeof module !== "undefined") {
  module.exports = {
    parseVtree, leafVariables, dimacsHeader, occurrences, crc32, zipStore,
    initialExpansion, expandBelow, layoutTree, nodeAt, sizesFor,
    projectedChoices, weightedChoices, threeCopies, UNSATISFIABLE, UNITS,
  };
}
