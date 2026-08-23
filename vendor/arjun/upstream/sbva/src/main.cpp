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

#include <cstdio>
#include <cstdlib>
#include <ios>
#include <iostream>
#include <vector>
#include <string>
#include "sbva.h"
#include "argparse.hpp"
#include "time_mem.h"

#if defined(__GNUC__) && defined(__linux__)
#include <cfenv>
#endif

using std::string;
using std::cout;
using std::cerr;
using std::endl;
using std::vector;

using namespace SBVA;

auto run_bva(FILE *fin, FILE *fout, FILE *fproof, Tiebreak tiebreak, Config& common) {
    CNF f;
    f.parse_cnf(fin, common);
    f.run(tiebreak);
    auto ret = f.to_cnf(fout);
    if (fproof != nullptr) f.to_proof(fproof);
    return ret;
}

argparse::ArgumentParser program = argparse::ArgumentParser("sbva");
int main(int argc, char **argv) {
    Config config;
    FILE *fproof = nullptr;
    Tiebreak tiebreak = Tiebreak::ThreeHop;

    program.add_argument("-v", "--verb")
        .action([&](const auto& a) {config.verbosity = std::atoi(a.c_str());})
        .default_value(config.verbosity)
        .help("Enable tracing");
    program.add_argument("-p", "--proof")
        .action([&](const auto& a) {
                config.generate_proof = true;
                fproof = fopen(a.c_str(), "w");
                if (fproof == nullptr) {
                std::cerr << "Error: Could not open file " << a << " for reading" << endl;
                }
        })
        .help("Emit proof file here");
    program.add_argument("-s", "--steps")
        .action([&](const auto& a) {config.steps = 1e6 * std::atoll(a.c_str());})
        .default_value(config.steps)
        .help("Number of computation steps to do");
    program.add_argument("-m", "--maxreplace")
        .action([&](const auto& a) {config.max_replacements = std::atoi(a.c_str());})
        .default_value(config.max_replacements)
        .help("Maximum number of replacements to do. 0 = no limit");
    program.add_argument("-n", "--normal")
        .action([&](const auto&) {tiebreak = Tiebreak::None;})
        .flag()
        .help("Use original BVA tie-break. Runs BVA instead of SBVA");
    program.add_argument("--clscutoff")
        .action([&](const auto& a) {config.matched_cls_cutoff = std::atoi(a.c_str());})
        .help("Matched clauses cutoff. The larger, the larger the gain must be to perform BVA");
    program.add_argument("--litscutoff")
        .action([&](const auto& a) {config.matched_lits_cutoff = std::atoi(a.c_str());})
        .help("Matched literals cutoff. The larger, the larger the gain must be to perform BVA");
    program.add_argument("-c", "--countpreserve")
        .action([&](const auto&) {config.preserve_model_cnt = true;})
        .flag()
        .help("Preserve model count. Adds additional clauses but allows the tool to be used in propositional model ");
    program.add_argument("files").remaining().help("input file and output file");


    #if defined(__GNUC__) && defined(__linux__)
    feenableexcept(FE_INVALID   | FE_DIVBYZERO | FE_OVERFLOW);
    #endif

    //Reconstruct the command line so we can emit it later if needed
    string command_line;
    for(int i = 0; i < argc; i++) {
        command_line += string(argv[i]);
        if (i+1 < argc) command_line += " ";
    }
    cout << "c SBVA Version: " << SBVA::get_version_sha1() << endl;
    cout << "c SBVA compilation env: " << SBVA::get_compilation_env() << endl;
    cout << "c executed with command line: " << command_line << endl;

    try {
        program.parse_args(argc, argv);
        if (program.is_used("--help")) {
            cout
            << "Structured Bounded Variable Addition CNF transformer" << endl << endl
            << "sbva [options] [inputfile] [outputfile]" << endl;
            cout << program << endl;
            std::exit(0);
        }
    }
    catch (const std::exception& err) {
        std::cerr << err.what() << std::endl;
        std::cerr << program;
        exit(-1);
    }

    FILE *fin = stdin;
    FILE *fout = stdout;

    //parsing the input
    vector<std::string> files;
    try {
        files = program.get<std::vector<std::string>>("files");
        if (files.size() > 2) {
            cout << "ERROR: you can only give at most two files, input and an output file" << endl;
            exit(-1);
        }
    } catch (std::logic_error& e) {
        //exit(-1);
    }

    auto my_time = cpuTime();
    if (!files.empty()) {
        const string in_fname = files[0];
        fin = fopen(in_fname.c_str(), "r");
        if (fin == nullptr) {
            cerr << "Error: Could not open file " << in_fname << " for reading" << endl;
            return 1;
        }
        cout << "c reading CNF from file " << in_fname << endl;
    } else cout << "c reading from stdin..." << endl;

    if (files.size() >= 2) {
        const string out_fname = files[1];
        fout = fopen(out_fname.c_str(), "w");
        if (fout == nullptr) {
            cerr << "Error: Could not open file " << out_fname << " for writing" << endl;
            return 1;
        }
        cout << "c writing transformed CNF to file " << out_fname << endl;
    } else cout << "c writing transformed CNF to stdout..." << endl;

    auto ret = run_bva(fin, fout, fproof, tiebreak, config);
    cout << "c SBVA Finished. Num vars now: " << ret.first << " num cls: " << ret.second << endl;
    cout << "c steps remainK: " << std::setprecision(2) << std::fixed << (double)config.steps/1000.0
           << " Timeout: " << (config.steps <= 0 ? "Yes" : "No")
           << " T: " << std::setprecision(2) << std::fixed
           << (cpuTime() - my_time)
           << endl;
}
