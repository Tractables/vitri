/******************************************
Copyright (C) 2023 Kenji Hashimoto

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

#pragma once

#include <vector>
#include <algorithm>
#include "bitset.hpp"
using std::vector;

namespace TWD {

class Graph {
public:
  Graph() : nodes(0), edges(0) { }
  Graph(int vars);

  void init(int n);
  void clear();

  int numNodes() const { return nodes; }
  int numEdges() const { return edges; }

  const vector<vector<int>>& get_adj_list() const;
  void addEdge(int v1, int v2);
  void contract(int v, int max_edges);
  bool hasEdge(int v1, int v2) { return adj_mat[v1].Get(v2); }
  const vector<int>& Neighbors(int v) const { return adj_list[v]; }
  bool isConnected() const {
    if (nodes == 0) return true;
    vector<int> visited(nodes, 0);
    auto dfs = [&](const int v, auto&& dfs_ref) -> void {
      visited[v] = 1;
      for (int u : adj_list[v]) {
        if (!visited[u]) dfs_ref(u, dfs_ref);
      }
    };

    dfs(0, dfs);
    for (int i = 0; i < nodes; i++) if (!visited[i]) return false;
    return true;
  }

protected:
  int nodes;
  int edges;
  vector<vector<int>> adj_list;
  vector<sspp::Bitset> adj_mat;
};

class TreeDecomposition : public Graph {
public:
  TreeDecomposition();

  void initBags() { bags.clear(); bags.resize(nodes); }
  vector<vector<int>>& Bags() { return bags; }
  bool inBag(int v, int x) const;
  void setWidth(int width) { tw = width; }
  int width() const { return tw; }
  void setNumGraphNodes(int n) { gnodes = n; }
  int centroid(int verb = 0);
  vector<int> distanceFromCentroid();
  double start_time;

private:
  void sortBags() {
    for (auto& bag: bags) std::sort(bag.begin(), bag.end());
  }
  int findCentroid(int v, int parent, int& centroid) const;
  void computeDistance(int v, int parent, int depth, vector<int>& distance);

  vector<vector<int>> bags;
  int tw;
  int gnodes;
  int cent = -1;
};
}
