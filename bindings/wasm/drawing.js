// The laid-out tree drawn two ways from one list of shapes: on a canvas for
// the page, and as an SVG document for saving. `sceneShapes` decides what is
// drawn and where; the two renderers only put the shapes down.
//
// A scene holds the tree, its layout and places (`x`), the sizes, which nodes
// are expanded, the component of every node (or null when components are not
// marked), the selected node and `onPath`, which marks the nodes from the
// selection up to the root.

import { LEAF_HEIGHT, NODE_RADIUS, ROW, STUB_HEIGHT } from "./layout.js";
import { isLeaf, isUnder } from "./vtree.js";

// Strokes and fonts in world units, shared by both renderers.
const STYLE = {
  edge: 1.2,
  pick: 3,
  leafStroke: 1.4,
  leafRadius: 4,
  stubStroke: 1.2,
  stubFill: 0.18,
  font: 12,
  stubFont: 10,
  // The stub's leaf count sits this far above the triangle's base.
  stubLabelRise: 7,
};

// A node's colour: its component's, grey for a node that joins components
// or a free variable, the accent when components are not marked.
function colorOf(scene, colors, node) {
  if (scene.component === null) return colors.accent;
  const c = scene.component[node];
  return c >= 0 ? colors.components[c % colors.components.length] : colors.muted;
}

// What to draw for a scene: edges, in two lists, the ones on the path to the
// selection and the rest, each as {xp, yp, xc, yc} from parent to child with
// the turn halfway down; and nodes, as leaves {left, top, width, height,
// dashed, label} and internal nodes {stub, label} where `stub` is the
// half-width of the triangle of a collapsed subtree or null. Every shape has
// its node, its centre `x`, `y` and `color`.
//
// `bounds` is the world rectangle to draw, and `scale` the pixels per world
// unit, which decides how much is legible: below a few pixels a slot, nodes
// are left out, and a subtree narrower than a pixel is its edge alone, since
// its own edges would all land on that pixel. The drawn nodes are in
// preorder, so such a subtree is the run of nodes after its root. `labels`
// says whether text would be legible.
export function sceneShapes(scene, colors, bounds, scale) {
  const { tree, layout, x, sizes, component, expanded, selected, onPath } = scene;
  const { x0, y0, x1, y1 } = bounds;
  const shapes = sizes.slot * scale >= 5;
  const edges = [];
  const picked = [];
  const nodes = [];
  for (let i = 0; i < layout.drawn; i++) {
    const node = layout.nodes[i];
    const up = tree.parent[node];
    const cx = x[node];
    const cy = tree.depth[node] * ROW;
    if (up >= 0) {
      const xp = x[up];
      if (!(Math.max(cx, xp) < x0 || Math.min(cx, xp) > x1 || cy < y0 || cy - ROW > y1)) {
        (selected >= 0 && onPath[node] === 1 ? picked : edges).push({ xp, yp: cy - ROW, xc: cx, yc: cy });
      }
    }
    if (shapes && !(cx < x0 - sizes.stub || cx > x1 + sizes.stub || cy < y0 - STUB_HEIGHT - LEAF_HEIGHT || cy > y1 + LEAF_HEIGHT)) {
      const color = colorOf(scene, colors, node);
      if (isLeaf(tree, node)) {
        nodes.push({
          node, x: cx, y: cy, color,
          left: cx - sizes.leaf / 2, top: cy - LEAF_HEIGHT / 2, width: sizes.leaf, height: LEAF_HEIGHT,
          dashed: component !== null && component[node] === -2,
          label: String(tree.variable[node]),
        });
      } else {
        const collapsed = expanded[node] === 0;
        nodes.push({ node, x: cx, y: cy, color, stub: collapsed ? sizes.stub / 2 - 5 : null, label: String(tree.leaves[node]) });
      }
    }
    if (!isLeaf(tree, node) && tree.leaves[node] * sizes.slot * scale < 1) {
      while (i + 1 < layout.drawn && isUnder(tree, layout.nodes[i + 1], node)) i += 1;
    }
  }
  return { edges, picked, nodes, shapes, labels: scale >= 0.55 };
}

// Draws the scene on a 2D context whose transform the caller has set to
// world units, then the selection ring.
export function drawScene(context, scene, colors, bounds, scale) {
  const { edges, picked, nodes, shapes, labels } = sceneShapes(scene, colors, bounds, scale);
  const px = 1 / scale;
  const trace = (list) => {
    const path = new Path2D();
    for (const { xp, yp, xc, yc } of list) {
      path.moveTo(xp, yp);
      path.lineTo(xp, yp + ROW / 2);
      path.lineTo(xc, yp + ROW / 2);
      path.lineTo(xc, yc);
    }
    return path;
  };
  context.lineJoin = "round";
  context.strokeStyle = colors.edge;
  context.lineWidth = Math.max(STYLE.edge, STYLE.edge * px);
  context.stroke(trace(edges));
  if (picked.length > 0) {
    context.strokeStyle = colors.pick;
    context.lineWidth = Math.max(STYLE.pick, STYLE.pick * px);
    context.stroke(trace(picked));
  }

  context.font = `${STYLE.font}px ${colors.mono}`;
  context.textAlign = "center";
  context.textBaseline = "middle";
  for (const shape of nodes) {
    if (shape.stub === undefined) {
      context.beginPath();
      roundedRect(context, shape.left, shape.top, shape.width, shape.height, STYLE.leafRadius);
      context.fillStyle = colors.card;
      context.fill();
      context.lineWidth = Math.max(STYLE.leafStroke, STYLE.edge * px);
      context.strokeStyle = shape.color;
      if (shape.dashed) context.setLineDash([3, 2]);
      context.stroke();
      context.setLineDash([]);
      if (labels) {
        context.fillStyle = colors.ink;
        context.fillText(shape.label, shape.x, shape.y + 0.5);
      }
    } else {
      if (shape.stub !== null) {
        context.beginPath();
        context.moveTo(shape.x, shape.y);
        context.lineTo(shape.x - shape.stub, shape.y + STUB_HEIGHT);
        context.lineTo(shape.x + shape.stub, shape.y + STUB_HEIGHT);
        context.closePath();
        context.globalAlpha = STYLE.stubFill;
        context.fillStyle = shape.color;
        context.fill();
        context.globalAlpha = 1;
        context.lineWidth = Math.max(STYLE.stubStroke, STYLE.stubStroke * px);
        context.strokeStyle = shape.color;
        context.stroke();
        if (labels) {
          context.font = `${STYLE.stubFont}px ${colors.mono}`;
          context.fillStyle = colors.ink;
          context.fillText(shape.label, shape.x, shape.y + STUB_HEIGHT - STYLE.stubLabelRise);
          context.font = `${STYLE.font}px ${colors.mono}`;
        }
      }
      context.beginPath();
      context.arc(shape.x, shape.y, NODE_RADIUS, 0, 2 * Math.PI);
      context.fillStyle = shape.color;
      context.fill();
    }
  }

  const { tree, x, sizes, selected } = scene;
  if (selected < 0) return;
  const cx = x[selected];
  const cy = tree.depth[selected] * ROW;
  context.lineWidth = STYLE.pick * px * (shapes ? 1 : 1.5);
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

// The whole scene as an SVG document: every drawn node, labels included, and
// the path to the selection, without the selection ring.
export function sceneSvg(scene, colors, title) {
  const { layout } = scene;
  const everything = { x0: -Infinity, y0: -Infinity, x1: Infinity, y1: Infinity };
  const { edges, picked, nodes } = sceneShapes(scene, colors, everything, 1);
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
  const trace = (list) => list.map(({ xp, yp, xc, yc }) => `M${r(xp)} ${yp}V${yp + ROW / 2}H${r(xc)}V${yc}`).join("");
  const svg = make("svg", { xmlns: ns, width, height, viewBox: `0 0 ${width} ${height}` });
  make("title", {}, svg).textContent = title;
  make("rect", { width, height, fill: colors.card }, svg);
  const world = make("g", { transform: `translate(${pad} ${pad + LEAF_HEIGHT / 2})` }, svg);
  make("path", { d: trace(edges) || "M0 0", fill: "none", stroke: colors.edge, "stroke-width": STYLE.edge }, world);
  if (picked.length > 0) make("path", { d: trace(picked), fill: "none", stroke: colors.pick, "stroke-width": STYLE.pick }, world);
  const font = { "font-family": colors.mono, "font-size": STYLE.font, "text-anchor": "middle", "dominant-baseline": "central" };
  for (const shape of nodes) {
    const cx = r(shape.x);
    if (shape.stub === undefined) {
      const group = make("g", {}, world);
      make("rect", {
        x: r(shape.left), y: shape.top, width: r(shape.width), height: shape.height, rx: STYLE.leafRadius,
        fill: colors.card, stroke: shape.color, "stroke-width": STYLE.leafStroke,
        ...(shape.dashed ? { "stroke-dasharray": "3 2" } : {}),
      }, group);
      make("text", { x: cx, y: shape.y, fill: colors.ink, ...font }, group).textContent = shape.label;
    } else {
      if (shape.stub !== null) {
        make("path", {
          d: `M${cx} ${shape.y}L${r(shape.x - shape.stub)} ${shape.y + STUB_HEIGHT}H${r(shape.x + shape.stub)}Z`,
          fill: shape.color, "fill-opacity": STYLE.stubFill, stroke: shape.color, "stroke-width": STYLE.stubStroke,
        }, world);
        make("text", {
          x: cx, y: shape.y + STUB_HEIGHT - STYLE.stubLabelRise, fill: colors.ink, ...font, "font-size": STYLE.stubFont,
        }, world).textContent = shape.label;
      }
      make("circle", { cx, cy: shape.y, r: NODE_RADIUS, fill: shape.color }, world);
    }
  }
  return new XMLSerializer().serializeToString(svg);
}
