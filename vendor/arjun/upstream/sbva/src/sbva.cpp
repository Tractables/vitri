/******************************************
Copyright (C) 2023 Andrew Haberlandt, Harrison Green, Marijn J.H. Heule
              2024 Changes, maybe bugs by Mate Soos

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
***********************************************/

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <queue>
#include <vector>
#include <algorithm>
#include <unordered_set>
#include <tuple>
#include <set>
#include <iomanip>

#include <cstdio>
#include <utility>

#include <Eigen/SparseCore>
#include "murmur.h"
#include "sbva.h"
#include "GitSHA1.hpp"
#include "getline.h"

using namespace std;

namespace SBVAImpl {

struct Clause {
    bool deleted;
    vector<int> lits;
    mutable uint32_t hash = 0;

    Clause() {
        deleted = false;
    }

    void print(const std::string extra = "") {
        if (deleted) {
            cout << extra << "DELETED: ";
        } else cout << extra;
        for (int lit : lits) cout << lit << " ";
        cout << endl;
    }

    uint32_t hash_val() const {
        if (hash == 0) {
            hash = murmur3_vec((uint32_t *) lits.data(), lits.size(), 0);
        }
        return hash;
    }

    bool operator==(const Clause &other) const {
        if (lits.size() != other.lits.size()) {
            return false;
        }
        for (size_t i = 0; i < lits.size(); i++) {
            if (lits[i] != other.lits[i]) {
                return false;
            }
        }
        return true;
    }
};


struct ProofClause {
    bool is_addition;
    vector<int> lits;

    ProofClause(bool _is_addition, vector<int> _lits) {
        this->is_addition = _is_addition;
        this->lits = _lits;
    }
};


struct ClauseHash {
    size_t operator()(const Clause &c) const {
        return c.hash_val();
    }
};

struct ClauseCache {
    unordered_set<Clause, ClauseHash> clauses;

    ClauseCache() = default;

    void add(Clause *c) {
        clauses.insert(*c);
    }

    bool contains(Clause *c) {
        return clauses.find(*c) != clauses.end();
    }
};

uint32_t lit_index(int32_t lit) {
    return (lit > 0 ? lit * 2 - 2 : -lit * 2 - 1);
}

uint32_t sparsevec_lit_idx(int32_t lit) {
    return (lit > 0 ? lit - 1: -lit - 1);
}

uint32_t sparcevec_lit_for_idx(int32_t lit) {
    return (lit + 1);
}

int reduction(int lits, int clauses) {
    return (lits * clauses) - (lits + clauses);
}

class Formula {
public:
    ~Formula() {
        delete cache;
    }

    Formula(SBVA::Config& _config) : config(_config) { }

    void init_cnf(uint32_t _num_vars) {
        num_vars = _num_vars;
        lit_count_adjust.resize(num_vars * 2);
        lit_to_clauses.resize(num_vars * 2);
        adjacency_matrix_width = num_vars * 4;
        adjacency_matrix.resize(num_vars);
        found_header = true;
        curr_clause = 0;
        assert(cache == nullptr);
        cache = new ClauseCache;
    }

    void add_cl(const vector<int>& cl_lits) {
        assert(found_header);
        clauses.push_back(Clause());
        assert(curr_clause == clauses.size()-1);

        for(const auto& lit: cl_lits) {
            assert(lit != 0);
            if ((uint32_t)abs(lit) > num_vars) {
                fprintf(stderr, "Error: CNF file has a variable that is greater than the number of variables specified in the header\n");
                exit(1);
            }
            config.steps--;
            clauses[(curr_clause)].lits.push_back(lit);
        }

        sort(clauses[(curr_clause)].lits.begin(), clauses[(curr_clause)].lits.end());

        auto *cls = &clauses[(curr_clause)];
        if (cache->contains(cls)) {
            cls->deleted = true;
            adj_deleted++;
        } else {
            cache->add(cls);
            for (auto l : clauses[(curr_clause)].lits) {
                config.steps--;
                lit_to_clauses[lit_index(l)].push_back(curr_clause);
            }
        }

        curr_clause++;
        num_clauses = curr_clause;
    }

    void finish_cnf() {
        delete cache;
        cache = nullptr;
        for (size_t i=1; i<=num_vars; i++) {
            update_adjacency_matrix(i);
        }
    }

    void read_cnf(FILE *fin) {
        char *line = nullptr;
        size_t len = 0;

        assert(cache == nullptr);
        cache = new ClauseCache;

        curr_clause = 0;
        while (getline2(&line, &len, fin) != -1) {
            if (len == 0) {
                continue;
            }
            if (line[0] == 'c') {
                continue;
            } else if (line[0] == 'p') {
                sscanf(line, "p cnf %lu %lu", &num_vars, &num_clauses);
                clauses.resize(num_clauses);
                lit_to_clauses.resize(num_vars * 2);
                lit_count_adjust.resize(num_vars * 2);
                adjacency_matrix_width = num_vars * 4;
                adjacency_matrix.resize(num_vars);
                found_header = true;
            } else {
                if (!found_header) {
                    fprintf(stderr, "Error: CNF file does not have a header\n");
                    exit(1);
                }

                if (curr_clause >= num_clauses && line[0] == 0) {
                    fprintf(stderr, "Error: CNF file has more clauses than specified in header\n");
                    exit(1);
                }

                int lit = 0;
                char *curr = line;
                while (sscanf(curr, "%d", &lit) > 0) {
                    if (lit == 0) {
                        break;
                    }
                    if ((uint32_t)abs(lit) > num_vars) {
                        fprintf(stderr, "Error: CNF file has a variable that is greater than the number of variables specified in the header\n");
                        exit(1);
                    }
                    config.steps--;
                    clauses[(curr_clause)].lits.push_back(lit);
                    curr = strchr(curr, ' ');
                    curr++;
                }

                sort(clauses[(curr_clause)].lits.begin(), clauses[(curr_clause)].lits.end());

                auto *cls = &clauses[(curr_clause)];
                if (cache->contains(cls)) {
                    cls->deleted = true;
                    adj_deleted++;
                } else {
                    cache->add(cls);
                    for (auto l : clauses[(curr_clause)].lits) {
                        config.steps--;
                        lit_to_clauses[lit_index(l)].push_back(curr_clause);
                    }
                }

                curr_clause++;
            }
        }
        free(line);
        delete cache;
        cache = nullptr;

        for (size_t i=1; i<=num_vars; i++) {
            update_adjacency_matrix(i);
        }
    }

    void update_adjacency_matrix(int lit) {
        int abslit = std::abs(lit);
        if (adjacency_matrix[sparsevec_lit_idx(abslit)].nonZeros() > 0) {
            // use cached version
            return;
        }
        Eigen::SparseVector<int> vec(adjacency_matrix_width);

        for (int cid : lit_to_clauses[lit_index(abslit)]) {
            config.steps--;
            Clause *cls = &clauses[cid];
            if (cls->deleted) continue;
            for (int v : cls->lits) {
                vec.coeffRef(sparsevec_lit_idx(v)) += 1;
            }
        }

        for (int cid : lit_to_clauses[lit_index(-abslit)]) {
            config.steps--;
            Clause *cls = &clauses[cid];
            if (cls->deleted) continue;
            for (int v : cls->lits) {
                vec.coeffRef(sparsevec_lit_idx(v)) += 1;
            }
        }

        adjacency_matrix[sparsevec_lit_idx(abslit)] = vec;
    }

    int tiebreaking_heuristic(int lit1, int lit2) {
        if (tmp_heuristic_cache_full.find(sparsevec_lit_idx(lit2)) != tmp_heuristic_cache_full.end()) {
            return tmp_heuristic_cache_full[sparsevec_lit_idx(lit2)];
        }
        int abs1 = std::abs(lit1);
        int abs2 = std::abs(lit2);
        update_adjacency_matrix(lit1);
        update_adjacency_matrix(lit2);

        auto& vec1 = adjacency_matrix[sparsevec_lit_idx(abs1)];
        auto& vec2 = adjacency_matrix[sparsevec_lit_idx(abs2)];

        int total_count = 0;
        for (int *var_ptr = vec2.innerIndexPtr(); var_ptr < vec2.innerIndexPtr() + vec2.nonZeros(); var_ptr++) {
            config.steps--;
            int var = sparcevec_lit_for_idx(*var_ptr);
            int count = vec2.coeffRef(sparsevec_lit_idx(var));
            update_adjacency_matrix(var);
            auto& vec3 = adjacency_matrix[sparsevec_lit_idx(var)];
            total_count += count * vec3.dot(vec1);
        }
        tmp_heuristic_cache_full[sparsevec_lit_idx(lit2)] = total_count;
        return total_count;
    }

    auto to_cnf(FILE *fout) {
        fprintf(fout, "p cnf %lu %lu\n", num_vars, num_clauses - adj_deleted);
        for (size_t i = 0; i < num_clauses; i++) {
            if (clauses[(i)].deleted) {
                continue;
            }
            for (int lit : clauses[(i)].lits) {
                fprintf(fout, "%d ", lit);
            }
            fprintf(fout, "0\n");
        }
        return std::make_pair(num_vars, num_clauses-adj_deleted);
    }

    vector<int> get_cnf(uint32_t& ret_num_vars, uint32_t& ret_num_cls) {
        vector<int> ret;
        ret_num_cls = num_clauses - adj_deleted;
        ret_num_vars = num_vars;
        for (size_t i = 0; i < num_clauses; i++) {
            if (clauses[(i)].deleted) continue;
            for (int lit : clauses[(i)].lits) {
                ret.push_back(lit);
            }
            ret.push_back(0);
        }
        return ret;
    }

    void to_proof(FILE *fproof) {
        for (const auto & clause : proof) {
            if (!clause.is_addition) {
                fprintf(fproof, "d ");
            }
            for (int lit : clause.lits) {
                fprintf(fproof, "%d ", lit);
            }
            fprintf(fproof, "0\n");
        }
    }

    int least_frequent_not(Clause *clause, int var) {
        int lmin = 0;
        int lmin_count = 0;
        for (auto lit : clause->lits) {
            if (lit == var) {
                continue;
            }
            int count = lit_to_clauses[lit_index(lit)].size() + lit_count_adjust[lit_index(lit)];
            if (lmin == 0 || count < lmin_count) {
                lmin = lit;
                lmin_count = count;
            }
        }
        return lmin;
    }

    int real_lit_count(int lit) {
        return lit_to_clauses[lit_index(lit)].size() + lit_count_adjust[lit_index(lit)];
    }

    // Performs partial clause difference between clause and other, storing the result in diff.
    // Only the first max_diff literals are stored in diff.
    // Requires that clause and other are sorted.
    void clause_sub(Clause *clause, Clause *other, vector<int>& diff, uint32_t max_diff) {
        diff.resize(0);
        size_t idx_a = 0;
        size_t idx_b = 0;

        while (idx_a < clause->lits.size() && idx_b < other->lits.size() && diff.size() <= max_diff) {
            config.steps--;
            if (clause->lits[idx_a] == other->lits[idx_b]) {
                idx_a++;
                idx_b++;
            } else if (clause->lits[idx_a] < other->lits[idx_b]) {
                diff.push_back(clause->lits[idx_a]);
                idx_a++;
            } else {
                idx_b++;
            }
        }

        while (idx_a < clause->lits.size() && diff.size() <= max_diff) {
            diff.push_back(clause->lits[idx_a]);
            idx_a++;
        }
    }

    void run_sbva(SBVA::Tiebreak tiebreak_mode) {
        struct PairOp {
            bool operator()(const pair<int, int> &a, const pair<int, int> &b) {
                return a.first < b.first;
            }
        };

        // The priority queue keeps track of all the literals to evaluate for replacements.
        // Each entry is the pair (num_clauses, lit)
        priority_queue<pair<int,int>, vector< pair<int,int> >, PairOp> pq;

        // Add all of the variables from the original formula to the priority queue.
        for (size_t i = 1; i <= num_vars; i++) {
            pq.push(make_pair(real_lit_count(i), i));
            pq.push(make_pair(real_lit_count(-i), -i));
        }

        vector<int> matched_lits;
        vector<int>* matched_clauses(new vector<int>());
        vector<int> *matched_clauses_swap(new vector<int>());
        vector<int> *matched_clauses_id( new vector<int>());
        vector<int> *matched_clauses_id_swap(new vector<int>());
        matched_lits.reserve(10000);
        matched_clauses->reserve(10000);
        matched_clauses_swap->reserve(10000);
        matched_clauses_id->reserve(10000);
        matched_clauses_id_swap->reserve(10000);

        // Track the index of the matched clauses from every literal that is added to matched_lits.
        vector< tuple<int, int> > clauses_to_remove;

        // Used for computing clause differences
        vector<int> diff;

        // Keep track of the matrix of swaps that we can perform.
        // Each entry is of the form (literal, <clause index>, <index in matched_clauses>)
        //
        // For example, given the formula:
        // (A v E)  (A v F)  (A v G)  (A v H)
        // (B v E)  (B v F)  (B v G)  (B v H)
        // (C v E)  (C v F)           (C v H)
        // (D v E)  (D v F)
        //
        // We would start with the following matrix:
        // matched_entries:     (A, (A v E), 0)  (A, (A v F), 1)  (A, (A v G), 2)  (A, (A v H), 3)
        // matched_clauses_id:  0  1  2  3
        // matched_clauses:     (A v E)  (A v F)  (A v G)  (A v H)
        //
        // Then, when we add B to matched_lits, we would get:
        // matched_entries:     (A, (A v E), 0)  (A, (A v F), 1)  (A, (A v G), 2)  (A, (A v H), 3)
        //                      (B, (B v E), 0)  (B, (B v F), 1)  (B, (B v G), 2)  (B, (B v H), 3)
        // matched_clauses_id:  0  1  2  3
        // matched_clauses:     (A v E)  (A v F)  (A v G)  (A v H)
        //
        // Then, when we add C to matched_lits, we would get:
        // matched_entries:     (A, (A v E), 0)  (A, (A v F), 1)  (A, (A v G), 2)  (A, (A v H), 3)
        //                      (B, (B v E), 0)  (B, (B v F), 1)  (B, (B v G), 2)  (B, (B v H), 3)
        //                      (C, (C v E), 0)  (C, (C v F), 1)                   (C, (C v H), 3)
        // matched_clauses_id:  0  1  3
        // matched_clauses:     (A v E)  (A v F)  (A v H)
        //
        // Adding D to matched_lits would not result in a reduction so we stop here.
        //
        // The matched_clauses_id is then used as a filter to find the clauses to remove:
        //
        // to_remove:   (A v E)  (A v F)  (A v H)
        //              (B v E)  (B v F)  (B v H)
        //              (C v E)  (C v F)  (C v H)
        //
        vector< tuple<int, int, int> > matched_entries;

        // Keep a list of the literals that are matched so we can sort and count later.
        vector<int> matched_entries_lits;

        // Used for priority queue updates.
        unordered_set<int> lits_to_update;

        // Track number of replacements (new auxiliary variables).
        size_t num_replacements = 0;


        while (!pq.empty()) {
            // check timeout
            if (config.steps < 0 ) {
                if (config.verbosity)
                    cout << "c stopping SBVA due to timeout. time remainK: "
                        << std::setprecision(2) << std::fixed << config.steps/1000.0 << endl;
                delete matched_clauses;
                delete matched_clauses_swap;
                delete matched_clauses_id;
                delete matched_clauses_id_swap;
                return;
            }
            if (config.verbosity >= 2)
                cout << "c time remainK: "
                    << std::setprecision(2) << std::fixed << config.steps/1000.0 << endl;

            // check replacement limit
            if (config.max_replacements != 0 && num_replacements == config.max_replacements) {
                if (config.verbosity) {
                    cout << "Hit replacement limit (" << config.max_replacements << ")" << endl;
                }
                delete matched_clauses;
                delete matched_clauses_swap;
                delete matched_clauses_id;
                delete matched_clauses_id_swap;
                return;
            }

            matched_lits.clear();
            matched_clauses->clear();
            matched_clauses_id->clear();
            clauses_to_remove.clear();
            tmp_heuristic_cache_full.clear();

            // Get the next literal to evaluate.
            pair<int, int> p = pq.top();
            pq.pop();

            int var = p.second;
            int num_matched = p.first;

            if (num_matched == 0 || num_matched != real_lit_count(var)) {
                continue;
            }

            if (config.verbosity) {
                cout << "Trying " << var << " (" << num_matched << ")" << endl;
            }

            // Mlit := { l }
            matched_lits.push_back(var);

            // Mcls := F[l]
            for (size_t i = 0; i < lit_to_clauses[lit_index(var)].size(); i++) {
                config.steps--;
                int clause_idx = lit_to_clauses[lit_index(var)][i];
                if (!clauses[(clause_idx)].deleted) {
                    matched_clauses->push_back(clause_idx);
                    matched_clauses_id->push_back(i);
                    clauses_to_remove.push_back(make_tuple(clause_idx, i));
                }
            }

            while (1) {
                // P = {}
                matched_entries.clear();
                matched_entries_lits.clear();

                if (config.verbosity) {
                    cout << "Iteration, Mlit: ";
                    for (int matched_lit : matched_lits) {
                        cout << matched_lit << " ";
                    }
                    cout << endl;
                }

                // foreach C in Mcls
                for (size_t i = 0; i < matched_clauses->size(); i++) {
                    config.steps--;
                    int clause_idx = (*matched_clauses)[(i)];
                    int clause_id = (*matched_clauses_id)[(i)];
                    auto *clause = &clauses[(clause_idx)];

                    if (config.verbosity >= 3) {
                        cout << "  Clause " << clause_idx << " (" << clause_id << "): ";
                        clause->print();
                    }

                    // let lmin in (C \ {l}) be least occuring in F
                    int lmin = least_frequent_not(clause, var);
                    if (lmin == 0) {
                        continue;
                    }

                    // foreach D in F[lmin]
                    for (auto other_idx : lit_to_clauses[lit_index(lmin)]) {
                        config.steps--;
                        auto *other = &clauses[(other_idx)];
                        if (other->deleted) {
                            continue;
                        }

                        if (clause->lits.size() != other->lits.size()) {
                            continue;
                        }

                        // diff := C \ D (limited to 2)
                        clause_sub(clause, other, diff, 2);

                        // if diff = {l} then
                        if (diff.size() == 1 && diff[0] == var) {
                            // diff := D \ C (limited to 2)
                            clause_sub(other, clause, diff, 2);

                            // if diff = {lmin} then
                            auto lit = diff[0];

                            // TODO: potential performance improvement
                            bool found = false;
                            for (auto l : matched_lits) {
                                if (l == lit) {
                                    found = true;
                                    break;
                                }
                            }

                            // if lit not in Mlit then
                            if (!found) {
                                // Add to clause match matrix.
                                matched_entries.push_back(make_tuple(lit, other_idx, i));
                                matched_entries_lits.push_back(lit);
                            }
                        }
                    }
                }

                // lmax := most frequent literal in P

                config.steps-= matched_entries_lits.size();
                sort(matched_entries_lits.begin(), matched_entries_lits.end());

                int lmax = 0;
                int lmax_count = 0;

                std::vector<int> ties;
                ties.reserve(16);
                for (size_t i2 = 0; i2 < matched_entries_lits.size();) {
                    int lit = matched_entries_lits[i2];
                    int count = 0;

                    while (i2 < matched_entries_lits.size() && matched_entries_lits[i2] == lit) {
                        config.steps--;
                        count++;
                        i2++;
                    }

                    if (config.verbosity >= 3) {
                        cout << "  " << lit << " count: " << count << endl;
                    }

                    if (count > lmax_count) {
                        lmax = lit;
                        lmax_count = count;
                        ties.clear();
                        ties.push_back(lit);
                    } else if (count == lmax_count) {
                        ties.push_back(lit);
                    }
                }

                if (lmax == 0) {
                    break;
                }

                int prev_clause_count = matched_clauses->size();
                int new_clause_count = lmax_count;

                int prev_lit_count = matched_lits.size();
                int new_lit_count = prev_lit_count + 1;

                // if adding lmax to Mlit does not result in a reduction then stop
                int current_reduction = reduction(prev_lit_count, prev_clause_count);
                int new_reduction = reduction(new_lit_count, new_clause_count);

                if (config.verbosity) {
                    cout << "  lmax: " << lmax << " (" << lmax_count << ")" << endl;
                    cout << "  current_reduction: " << current_reduction << endl;
                    cout << "  new_reduction: " << new_reduction << endl;
                }

                if (new_reduction <= current_reduction) {
                    break;
                }

                // Break ties
                if (ties.size() > 1 && tiebreak_mode == SBVA::Tiebreak::ThreeHop) {
                    int max_heuristic_val = tiebreaking_heuristic(var, ties[0]);
                    for (size_t i=1; i<ties.size(); i++) {
                        config.steps--;
                        int h = tiebreaking_heuristic(var, ties[i]);
                        if (h > max_heuristic_val) {
                            max_heuristic_val = h;
                            lmax = ties[i];
                        }
                    }
                }


                // Mlit := Mlit U {lmax}
                matched_lits.push_back(lmax);

                // Mcls := Mcls U P[lmax]
                matched_clauses_swap->resize(lmax_count);
                matched_clauses_id_swap->resize(lmax_count);

                int insert_idx = 0;
                for (const auto& pair : matched_entries) {
                    config.steps--;
                    int lit = get<0>(pair);
                    if (lit != lmax) continue;

                    int clause_idx = get<1>(pair);
                    int idx = get<2>(pair);

                    (*matched_clauses_swap)[(insert_idx)] = (*matched_clauses)[(idx)];
                    (*matched_clauses_id_swap)[(insert_idx)] = (*matched_clauses_id)[(idx)];
                    insert_idx += 1;

                    clauses_to_remove.push_back(make_tuple(clause_idx, (*matched_clauses_id)[(idx)]));
                }

                swap(matched_clauses,matched_clauses_swap);
                swap(matched_clauses_id,matched_clauses_id_swap);

                if (config.verbosity) {
                    cout << "  Mcls: ";
                    for (int matched_clause : (*matched_clauses)) {
                        cout << matched_clause << " ";
                    }
                    cout << endl;
                    cout << "  Mcls_id: ";
                    for (int i : (*matched_clauses_id)) {
                        cout << i << " ";
                    }
                    cout << endl;
                }
            }

            if (matched_lits.size() == 1) {
                continue;
            }

            if (matched_lits.size() <= config.matched_lits_cutoff &&
                    matched_clauses->size() <= config.matched_cls_cutoff) {
                continue;
            }

            int matched_clause_count = matched_clauses->size();
            int matched_lit_count = matched_lits.size();

            if (config.verbosity) {
                cout << "  mlits: ";
                for (int matched_lit : matched_lits) {
                    cout << matched_lit << " ";
                }
                cout << endl;
                cout << "  mclauses:\n";
                for (int matched_clause : (*matched_clauses)) {
                    clauses[matched_clause].print("   -> ");
                }
                cout << endl;

                cout << "--------------------" << endl;
            }
            assert(lit_to_clauses.size() == num_vars*2);
            assert(lit_count_adjust.size() == num_vars*2);

            // Do the substitution
            num_vars += 1;
            int new_var = num_vars;

            // Prepare to add new clauses.
            uint32_t new_sz = num_clauses + matched_lit_count + matched_clause_count +
                (config.preserve_model_cnt ? 1 : 0);
            if (clauses.size() >= new_sz) clauses.resize(new_sz);
            else clauses.insert(clauses.end(), new_sz - clauses.size(), Clause());

            lit_to_clauses.insert(lit_to_clauses.end(), 2, vector<int>());
            lit_count_adjust.insert(lit_count_adjust.end(), 2, 0);
            if (sparsevec_lit_idx(new_var) >= adjacency_matrix_width) {
                // The vectors must be constructed with a fixed, pre-determined width.
                //
                // This is quite an annoying limitation, as it means we have to re-construct
                // all the vectors if we go above the width limit
                adjacency_matrix_width = num_vars * 2;
                adjacency_matrix.clear();
            }
            adjacency_matrix.resize(num_vars);

            // Add (f, lit) clauses.
            for (int i = 0; i < matched_lit_count; ++i) {
                config.steps--;
                int lit = matched_lits[(i)];
                int new_clause = num_clauses + i;

                auto cls = Clause();
                cls.lits.push_back(lit);
                cls.lits.push_back(new_var); // new_var is always largest value
                (clauses)[new_clause] = cls;

                lit_to_clauses[lit_index(lit)].push_back(new_clause);
                lit_to_clauses[lit_index(new_var)].push_back(new_clause);

                if (config.generate_proof) {
                    auto proof_lits = vector<int>();
                    proof_lits.push_back(new_var); // new_var needs to be first for proof
                    proof_lits.push_back(lit);
                    proof.push_back(ProofClause(true, proof_lits));
                }
            }

            // Add (-f, ...) clauses.
            for (int i = 0; i < matched_clause_count; ++i) {
                config.steps--;
                int clause_idx = (*matched_clauses)[i];
                auto new_clause = num_clauses + matched_lit_count + i;

                auto cls = Clause();
                cls.lits.push_back(-new_var); // -new_var is always smallest value
                lit_to_clauses[lit_index(-new_var)].push_back(new_clause);

                auto match_cls = clauses[(clause_idx)];
                for (auto mlit : match_cls.lits) {
                    if (mlit != var) {
                        cls.lits.push_back(mlit);
                        lit_to_clauses[lit_index(mlit)].push_back(new_clause);
                    }
                }
                clauses[new_clause] = cls;

                if (config.generate_proof) {
                    proof.push_back(ProofClause(true, cls.lits));
                }
            }

            // Preserving model count:
            //
            // The only case where we add a model is if both assignments for the auxiiliary variable satisfy the formula
            // for the same assignment of the original variables. This only happens if all(matched_lits) *AND*
            // all(matches_clauses) are satisfied.
            //
            // The easiest way to fix this is to add one clause that constrains all(matched_lits) => -f
            if (config.preserve_model_cnt) {
                int new_clause = num_clauses + matched_lit_count + matched_clause_count;
                auto cls = Clause();
                cls.lits.push_back(-new_var);
                for (int i = 0; i < matched_lit_count; ++i) {
                    int lit = (matched_lits)[i];
                    cls.lits.push_back(-lit);
                    lit_to_clauses[lit_index(-lit)].push_back(new_clause);
                }

                (clauses)[new_clause] = cls;
                lit_to_clauses[(lit_index(-new_var))].push_back(new_clause);

                if (config.generate_proof) {
                    proof.push_back(ProofClause(true, cls.lits));
                }
            }


            set<int> valid_clause_ids;
            for (int i = 0; i < matched_clause_count; ++i) {
                config.steps--;
                valid_clause_ids.insert((*matched_clauses_id)[i]);
            }

            // Remove the old clauses.
            int removed_clause_count = 0;
            lits_to_update.clear();

            for (auto to_remove : clauses_to_remove) {
                int clause_idx = get<0>(to_remove);
                int clause_id = get<1>(to_remove);

                if (valid_clause_ids.find(clause_id) == valid_clause_ids.end()) {
                    continue;
                }

                auto cls = &(clauses)[clause_idx];
                cls->deleted = true;
                removed_clause_count += 1;
                for (auto lit : cls->lits) {
                    config.steps--;
                    lit_count_adjust[lit_index(lit)] -= 1;
                    lits_to_update.insert(lit);
                }

                if (config.generate_proof) {
                    proof.push_back(ProofClause(false, cls->lits));
                }
            }

            adj_deleted += removed_clause_count;
            num_clauses += matched_lit_count + matched_clause_count + (config.preserve_model_cnt ? 1 : 0);

            // Update priorities.
            for (auto lit : lits_to_update) {
                // Q.push(lit);
                pq.push(make_pair(
                    real_lit_count(lit),
                    lit
                ));

                // Reset adjacency matrix
                adjacency_matrix[sparsevec_lit_idx(lit)] = Eigen::SparseVector<int>(adjacency_matrix_width);
            }

            // Q.push(new_var);
            pq.push(make_pair(
                lit_to_clauses[lit_index(new_var)].size() + (lit_count_adjust)[lit_index(new_var)],
                new_var
            ));

            // Q.push(-new_var);
            pq.push(make_pair(
                lit_to_clauses[lit_index(-new_var)].size() + (lit_count_adjust)[lit_index(-new_var)],
                -new_var
            ));

            // Q.push(var);
            pq.push(make_pair(
                lit_to_clauses[lit_index(var)].size() + (lit_count_adjust)[lit_index(var)],
                var
            ));

            num_replacements += 1;
        }
        delete matched_clauses;
        delete matched_clauses_swap;
        delete matched_clauses_id;
        delete matched_clauses_id_swap;
    }

private:
    bool found_header = false;
    size_t num_vars = 0;
    size_t num_clauses = 0;
    size_t curr_clause = 0;
    int adj_deleted = 0;
    vector<Clause> clauses;
    SBVA::Config& config;
    ClauseCache* cache = nullptr;

    // maps each literal to a vector of clauses that contain it
    vector< vector<int> > lit_to_clauses;
    vector<int> lit_count_adjust;

    uint32_t adjacency_matrix_width;
    vector< Eigen::SparseVector<int> > adjacency_matrix;
    map< int, int > tmp_heuristic_cache_full;

    // proof storage
    vector<ProofClause> proof;
};

}

namespace SBVA {
using namespace SBVAImpl;

CNF::CNF() = default;

CNF::~CNF() {
    Formula* f = (Formula*)data;
    delete f;
    f = nullptr;
}

void CNF::run(SBVA::Tiebreak t) {
    Formula* f = (Formula*)data;
    f->run_sbva(t);
}

std::pair<int, int> CNF::to_cnf(FILE* file) {
    Formula* f = (Formula*)data;
    return f->to_cnf(file);
}

void CNF::to_proof(FILE* file) {
    Formula* f = (Formula*)data;
    f->to_proof(file);
}

vector<int> CNF::get_cnf(uint32_t& ret_num_vars, uint32_t& ret_num_cls) {
    Formula* f = (Formula*)data;
    return f->get_cnf(ret_num_vars, ret_num_cls);
}


void CNF::init_cnf(uint32_t num_vars, Config& config) {
    assert(data == nullptr);
    Formula* f = new Formula(config);
    f->init_cnf(num_vars);
    data = (void*)f;
}

void CNF::add_cl(const std::vector<int>& cl_lits) {
    Formula* f = (Formula*)data;
    f->add_cl(cl_lits);
}

void CNF::finish_cnf() {
    Formula* f = (Formula*)data;
    f->finish_cnf();
}

void CNF::parse_cnf(FILE* file, Config& config) {
    assert(data == nullptr);
    Formula* f = new Formula(config);
    f->read_cnf(file);
    CNF cnf;
    data = (void*)f;
}

const char* get_version_tag() {
    return SBVAImpl::get_version_tag();
}
const char* get_version_sha1() {
    return SBVAImpl::get_version_sha1();
}
const char* get_compilation_env() {
    return SBVAImpl::get_compilation_env();
}

}
