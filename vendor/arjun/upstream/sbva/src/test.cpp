/******************************************
Copyright (C) 2024 Mate Soos

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

// a=1, b=2, c=3, d=4, e=5, f=6, g=7, h=8

// (A v E)  (A v F)  (A v G)  (A v H)
// (B v E)  (B v F)  (B v G)  (B v H)
// (C v E)  (C v F)           (C v H)
// (D v E)  (D v F)

#include "sbva.h"
#include <iostream>
using std::cout;
using std::endl;

int main() {
    SBVA::CNF cnf;
    SBVA::Config config;
    cnf.init_cnf(8, config);

    cnf.add_cl({1, 5});
    cnf.add_cl({2, 5});
    cnf.add_cl({3, 5});
    cnf.add_cl({4, 5});

    cnf.add_cl({1, 6});
    cnf.add_cl({2, 6});
    cnf.add_cl({3, 6});
    cnf.add_cl({4, 6});

    cnf.add_cl({1, 7});
    cnf.add_cl({2, 7});

    cnf.add_cl({1, 8});
    cnf.add_cl({2, 8});
    cnf.add_cl({3, 8});
    cnf.finish_cnf();
    cnf.run(SBVA::Tiebreak::ThreeHop);

    uint32_t num_vars;
    uint32_t num_cls;
    auto ret = cnf.get_cnf(num_vars, num_cls);
    cout << "p cnf " << num_vars << " " << num_cls << endl;
    for(const auto& l: ret) {
        cout << l;
        if (l == 0) cout << endl;
        else cout << " ";
    }

    return 0;
}
