// Readers for the two text formats the page takes apart: the `.vtree` format
// and DIMACS CNF. No DOM; the node smoke test imports this file too.

const isCount = (token) => /^\+?\d+$/.test(token);

// The `.vtree` text format the SDD library reads: a `vtree N` line, then N
// node lines, `L <id> <variable>` or `I <id> <left> <right>`, children before
// parents. The last node listed is the root. The checks are the crate's own:
// ids in range, every node listed, no variable on two leaves, and one tree.
//
// Nodes live in typed arrays indexed by the file's node id. `order` is a
// preorder walk, left child first; `pre[v]` is v's position in it and
// `last[v]` the position of the last node of v's subtree, so u is under v
// exactly when pre[v] <= pre[u] <= last[v]. Returns `{error}` for text that
// is not a vtree.
export function parseVtree(text) {
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

export const isLeaf = (tree, node) => tree.left[node] < 0;
export const isUnder = (tree, node, top) => tree.pre[top] <= tree.pre[node] && tree.pre[node] <= tree.last[top];

// Up to `limit` leaf variables of a subtree, left to right, and whether that
// was all of them.
export function leafVariables(tree, top, limit) {
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
export function dimacsHeader(text) {
  const match = /^p\s+cnf\s+(\d+)\s+(\d+)/m.exec(text.slice(0, 1 << 20));
  return match === null ? null : { variables: Number(match[1]), clauses: Number(match[2]) };
}

// How many clauses of a DIMACS text each variable occurs in, indexed by
// variable. A variable twice in one clause counts once.
export function occurrences(text, variables) {
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
