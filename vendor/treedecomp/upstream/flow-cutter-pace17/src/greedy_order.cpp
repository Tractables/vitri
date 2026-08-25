#include "greedy_order.hpp"
#include "array_id_func.hpp"
#include "tiny_id_func.hpp"
#include "permutation.hpp"
#include "heap.hpp"
#include <vector>

ArrayIDFunc<std::vector<int>> build_dyn_array(const ArrayIDIDFunc&tail, const ArrayIDIDFunc&head){
	const int node_count = tail.image_count();
	const int arc_count = tail.preimage_count();

	ArrayIDFunc<std::vector<int>> neighbors(node_count);

	for(int i=0; i<arc_count; ++i)
		neighbors[tail(i)].push_back(head(i));

	for(int i=0; i<node_count; ++i)
		std::sort(neighbors[i].begin(), neighbors[i].end());

	return neighbors; // NVRO
}

template<class T>
struct NullAssign{
public:
	NullAssign&operator=(const T&){
		return *this;
	}
};

template<class T>
struct CountOutputIterator{
	typedef T value_type;
	typedef int difference_type;
	typedef T*pointer;
	typedef T&reference;
	typedef std::output_iterator_tag iterator_category;

	NullAssign<T> operator*()const{
		return {};
	}

	CountOutputIterator(int&_n):
		n(&_n){}

	CountOutputIterator&operator++(){
		++*n;
		return *this;
	}

	CountOutputIterator&operator++(int){
		++*n;
		return *this;
	}

	int*n;
};


template<class Iter1, class Iter2, class Iter3, class T>
Iter3 set_union_and_remove_element(
	Iter1 a, Iter1 a_end,
	Iter2 b, Iter2 b_end,
	Iter3 out,
	const T&remove_element1, const T&remove_element2
){
	while(a != a_end && b != b_end){
		if(*a < *b){
			if(*a != remove_element1 && *a != remove_element2)
				*out++ = *a;
			++a;
		}else if(*a > *b){
			if(*b != remove_element1 && *b != remove_element2)
				*out++ = *b;
			++b;
		}else if(*a == *b){
			if(*a != remove_element1 && *a != remove_element2)
				*out++ = *a;
			++b;
			++a;
		}
	}

	while(a != a_end){
		if(*a != remove_element1 && *a != remove_element2)
			*out++ = *a;
		++a;
	}

	while(b != b_end){
		if(*b != remove_element1 && *b != remove_element2)
			*out++ = *b;
		++b;
	}

	return out;
}

// vitri: graph elements the two greedy pre-orderings sweep, counted in the same
// unit a caller's construction meter charges every other kind of graph work in:
// neighbourhood entries. These passes spend none of the restart loop's step
// budget, so before this counter existed their cost could only be modelled. A
// model of `arcs^2 / nodes` was short by around a hundredfold on a dense primal
// graph, where a single pass ran for seventeen seconds and was charged nothing.
//
// The counter is PER THREAD. It is read and reset by the construction that spent
// it, and a construction is a single thread's work, so a second construction
// running these passes at the same time in another thread neither adds to this
// one's reading nor spends its touch budget. A file static shared by both would
// make each build's charge depend on what the other thread happened to be doing,
// which is the dependence a metered build exists to remove.
static thread_local int64_t g_greedy_touches = 0;

int64_t greedy_order_take_touches(){
	int64_t v = g_greedy_touches;
	g_greedy_touches = 0;
	return v;
}

std::vector<int> contract_node(ArrayIDFunc<std::vector<int>>&graph, int node){
	std::vector<int>tmp;
	for(int x:graph(node)){
		g_greedy_touches += (int64_t)graph(node).size() + (int64_t)graph(x).size();
		tmp.clear();
		set_union_and_remove_element(
			graph(node).begin(), graph(node).end(),
			graph(x).begin(), graph(x).end(),
			std::back_inserter(tmp),
			node, 
			x
		);
		graph[x].swap(tmp);
	}

	return std::move(graph[node]);
}

int compute_number_of_shortcuts_added_if_contracted(const ArrayIDFunc<std::vector<int>>&graph, int node){
	int added = 0;
	for(int x:graph(node)){
		g_greedy_touches += (int64_t)graph(node).size() + (int64_t)graph(x).size();
		std::set_difference(
			graph(node).begin(), graph(node).end(),
			graph(x).begin(), graph(x).end(),
			CountOutputIterator<int>(added)
		);
		--added;
	}

	added /= 2;

	return added;
}


// vitri: has the abandonment deadline passed? `time_point::max()` is the "no
// deadline" sentinel and is tested first, so an untimed pass never reads the
// clock at all. A timed one reads it once per elimination step, against a
// contraction that is at least a neighbourhood scan, which is why there is no
// poll interval to tune.
static inline bool greedy_order_deadline_passed(std::chrono::steady_clock::time_point deadline){
	return deadline != std::chrono::steady_clock::time_point::max()
		&& std::chrono::steady_clock::now() >= deadline;
}

// vitri: has this pass spent its touch budget? The deadline above abandons a
// runaway pass at a point that depends on how loaded the machine is, which
// bounds the pass without reproducing it — the same graph can yield a different
// order on a second run. A touch budget abandons it at the same point every
// time. `budget <= 0` is the "no budget" sentinel, so a caller that does not
// meter its work never leaves the deadline behaviour.
static inline bool greedy_order_budget_spent(int64_t budget, int64_t start_touches){
	return budget > 0 && g_greedy_touches - start_touches >= budget;
}

ArrayIDIDFunc compute_greedy_min_degree_order(
	const ArrayIDIDFunc&tail, const ArrayIDIDFunc&head,
	std::chrono::steady_clock::time_point deadline,
	int64_t touch_budget
){
	const int64_t start_touches = g_greedy_touches;
	const int node_count = tail.image_count();

	auto g = build_dyn_array(tail, head);

	min_id_heap<int> q(node_count);

	for(int x=0; x<node_count; ++x)
		q.push(x, g(x).size());

	ArrayIDIDFunc order(node_count, node_count);
	int next_pos = 0;

	while(!q.empty()){
		// vitri: abandon WHOLE, never partial — a prefix of an elimination
		// order is not a permutation. See the contract in greedy_order.hpp.
		if(greedy_order_deadline_passed(deadline)
			|| greedy_order_budget_spent(touch_budget, start_touches))
			return ArrayIDIDFunc();

		auto x = q.pop();

		order[next_pos++] = x;

		for(auto y:contract_node(g, x)){
			q.push_or_set_key(y, g(y).size());
		}
	}

	return order; // NVRO
}

ArrayIDIDFunc compute_greedy_min_shortcut_order(
	const ArrayIDIDFunc&tail, const ArrayIDIDFunc&head,
	std::chrono::steady_clock::time_point deadline,
	int64_t touch_budget
){
	const int64_t start_touches = g_greedy_touches;
	const int node_count = tail.image_count();

	auto g = build_dyn_array(tail, head);

	min_id_heap<int> q(node_count);

	for(int x=0; x<node_count; ++x){
		// vitri: the priming loop is itself O(sum of deg^2) and on a dense
		// graph can outlast the deadline before a single node is eliminated,
		// so it is bounded too.
		if(greedy_order_deadline_passed(deadline)
			|| greedy_order_budget_spent(touch_budget, start_touches))
			return ArrayIDIDFunc();
		q.push(x, 100*compute_number_of_shortcuts_added_if_contracted(g,x) +  g(x).size());
	}

	ArrayIDIDFunc order(node_count, node_count);
	int next_pos = 0;

	while(!q.empty()){
		// vitri: abandon WHOLE, never partial — see above.
		if(greedy_order_deadline_passed(deadline)
			|| greedy_order_budget_spent(touch_budget, start_touches))
			return ArrayIDIDFunc();

		auto x = q.pop();

		order[next_pos++] = x;

		for(auto y:contract_node(g, x)){
			q.push_or_set_key(y, 100*compute_number_of_shortcuts_added_if_contracted(g,y) + g(y).size());
		}
	}

	return order; // NVRO
}

