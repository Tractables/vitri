// Which nodes of a tree are drawn and where, in world units: one row per
// depth, one slot per drawn leaf or collapsed subtree. Labels are set in a
// 12px monospace, about 7.2 units a character.

import { isLeaf } from "./vtree.js";

export const ROW = 64;
export const NODE_RADIUS = 6;
export const LEAF_HEIGHT = 20;
export const STUB_HEIGHT = 26;
const CHARACTER = 7.2;

export function sizesFor(tree) {
  const leaf = Math.max(24, String(tree.maxVariable).length * CHARACTER + 12);
  const slot = leaf + 8;
  const stub = Math.max(slot, String(tree.count).length * 6.2 + 20);
  return { leaf, slot, stub };
}

// Which internal nodes start expanded: breadth first from the root until the
// drawing would pass `budget` nodes, so the top of the tree shows evenly.
export function initialExpansion(tree, budget) {
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
export function expandBelow(tree, expanded, top, budget) {
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
export function layoutTree(tree, expanded, sizes, x) {
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
export function nodeAt(tree, layout, x, wx, wy, reach) {
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
