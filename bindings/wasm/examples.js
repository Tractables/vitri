// The row of examples: a key, which is how the address bar names one, the
// name on its button, and its maker. The tutorial formula and the competition
// formula are files beside the page; the others are made from the tutorial
// formula or written out here.

import { dimacsHeader } from "./vtree.js";

export const EXAMPLES = [
  ["choices", "tutorial formula, 12 variables", () => fetched("example.cnf")],
  ["choices-projected", "projected on the first slot", () => fetched("example.cnf").then(projectedChoices)],
  ["choices-weighted", "weighted", () => fetched("example.cnf").then(weightedChoices)],
  ["three-copies", "three independent copies", () => fetched("example.cnf").then(threeCopies)],
  ["unsatisfiable", "unsatisfiable", () => UNSATISFIABLE],
  ["units", "unit clauses only", () => UNITS],
  ["mc2023-track1-008", "competition formula, 58 variables", () => fetched("mc2023_track1_008.reduced.cnf")],
];

export const UNSATISFIABLE = [
  "c no assignment to variables 1 and 2 satisfies the first four clauses",
  "p cnf 4 5",
  "1 2 0",
  "-1 2 0",
  "1 -2 0",
  "-1 -2 0",
  "3 4 0",
  "",
].join("\n");

export const UNITS = ["c every clause is a unit", "p cnf 4 4", "1 0", "-2 0", "3 0", "-4 0", ""].join("\n");

// This file's version stamp (see README.md), passed on to the files it
// fetches.
const stamp = new URL(import.meta.url).search;

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
export function projectedChoices(text) {
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
export function weightedChoices(text) {
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
export function threeCopies(text) {
  const { variables, clauses } = clausesOf(text);
  const lines = ["c three copies of the tutorial formula and one unconstrained variable", `p cnf ${3 * variables + 1} ${3 * clauses.length}`];
  for (let copy = 0; copy < 3; copy++) {
    for (const clause of clauses) lines.push(renumbered(clause, copy * variables));
  }
  return dimacs(lines);
}
