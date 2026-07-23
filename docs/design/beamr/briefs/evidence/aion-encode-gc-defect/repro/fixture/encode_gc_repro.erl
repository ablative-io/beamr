%% AION-ENCODE-GC-DEFECT signature repro fixture.
%%
%% Reproduces the heap neighborhood of the L01 crash (aion workflow
%% 2062659a-afd4-4b9d-820c-d675bd29d5ed, 2026-07-23 19:31:42, badarg in
%% json:encode_binary at the fix_cycle fork on beamr 0.16.0): a
%% report-shaped, bool-rich structure — acceptance claims as 2-field
%% records, a gate outcome with pass=false and per-run passed booleans
%% (true/true/false), three ~16 KB strings riding inside the same encode
%% (diagnostics, diff, test output_tail) — built fresh each iteration so
%% ProcBin allocations sit beside false-headed cons cells, then encoded
%% string-by-string via json:encode_binary.
%%
%% All boolean lists are built DYNAMICALLY (computed comparisons, runtime
%% recursion) so erlc cannot fold them into constant-pool literals: the
%% C1 trigger needs `[false | _]` cons cells allocated on the process
%% heap. A bounded accumulator of gate flags persists across iterations
%% so promoted false-headed conses are present for old-generation walks
%% as well as young.
%%
%% The fixture never catches: a badarg crashes the process and the
%% harness reports the exit reason. RED (0.16.0-locked) = abnormal exit,
%% badarg, partway through the loop. GREEN (0.16.2-locked) = normal exit
%% returning {completed, N}.
-module(encode_gc_repro).
-export([main/0, main/1]).

main() -> main(25000).

main(N) ->
    loop(0, N, []).

loop(N, N, Acc) ->
    {completed, N, length(Acc)};
loop(I, N, Acc) ->
    Claims = claims(I),
    Gate = gate(I),
    Flags = run_flags(runs_of(Gate)),
    Alt = alt_list(I, 12),
    Encoded = encode_report(summary(I), Claims, Gate),
    AltEncoded = encode_alt(Alt),
    NextAcc = keep_bounded([pass_of(Gate) | append_flags(Flags, Acc)]),
    consume(Encoded),
    consume(AltEncoded),
    loop(I + 1, N, NextAcc).

%% Alternating [Flag, Bin, Flag, Bin, ...] with runtime-computed false
%% flags: in right-to-left list construction each false-headed cons is
%% allocated immediately after a binary-headed cons, so the misread
%% walk's zero-write lands on a binary term slot — the production
%% expression (badarg at the encode) rather than an arbitrary-address
%% free. The encode below is POSITIONAL and unguarded, like the real
%% dev_report_to_json path: it trusts the shape it built.
alt_list(_I, 0) -> [];
alt_list(I, K) ->
    [K rem 1 =:= 1, tag(I + K, <<"claim text riding beside booleans">>)
     | alt_list(I, K - 1)].

encode_alt([]) -> [];
encode_alt([_Flag, Bin | Rest]) ->
    [json:encode_binary(Bin) | encode_alt(Rest)].

%% --- report shape -----------------------------------------------------

summary(I) ->
    Seed = <<"Resolved the repeated mechanical gate failure without changing "
             "the brief diff: the gate runner now has the CI-pinned toolchain "
             "directly on its PATH, and the raw battery passes in order. ">>,
    tag(I, Seed).

%% Eight 2-field claim records, criterion/how strings in the observed
%% 100-500 byte band.
claims(I) ->
    claims(I, 8).

claims(_I, 0) -> [];
claims(I, K) ->
    Criterion = tag(I + K,
        <<"Every document in scope states the default store correctly; zero "
          "remaining stale claims (grep evidence required); enumerations "
          "list all variants exactly as defined by the source of record. ">>),
    How = tag(I * K,
        <<"The accepted implementation diff remains unchanged: all scoped "
          "documents verified against current source, the focused regex "
          "audit found no stale claim, and the raw full workspace battery "
          "passes on the same runner environment the pipeline uses. ">>),
    [{claim, Criterion, How} | claims(I, K - 1)].

%% Gate outcome mirroring L01's final crash-round gate: pass=false,
%% three runs passed true/true/false, ~16 KB diagnostics/diff and a
%% ~16 KB failing-run output tail. Exit codes are computed so `passed`
%% booleans are runtime values, never literals.
gate(I) ->
    Diagnostics = big_string(I, $d),
    Diff = big_string(I + 1, $f),
    TestTail = big_string(I + 2, $t),
    Runs = [
        {run, <<"fmt">>, exit_code(I, 0), <<>>},
        {run, <<"clippy">>, exit_code(I, 0),
         <<"    Finished `dev` profile [unoptimized + debuginfo]\n">>},
        {run, <<"test">>, exit_code(I, 101), TestTail}
    ],
    Pass = all_zero(Runs),
    {gate_outcome, Pass, Runs, Diff, Diagnostics}.

pass_of({gate_outcome, Pass, _Runs, _Diff, _Diag}) -> Pass.
runs_of({gate_outcome, _Pass, Runs, _Diff, _Diag}) -> Runs.

exit_code(I, Code) -> Code + (I - I).

all_zero([]) -> true;
all_zero([{run, _Name, 0, _Tail} | Rest]) -> all_zero(Rest);
all_zero([{run, _Name, _Code, _Tail} | _Rest]) -> false.

%% `[Passed | _]` cons cells with runtime-computed boolean heads — the
%% C1 collision shape, one per run, false for the failing test run.
run_flags([]) -> [];
run_flags([{run, _Name, Code, _Tail} | Rest]) ->
    [Code =:= 0 | run_flags(Rest)].

append_flags([], Acc) -> Acc;
append_flags([F | Rest], Acc) -> [F | append_flags(Rest, Acc)].

keep_bounded(Acc) -> keep_bounded(Acc, 300, []).

keep_bounded([], _K, Kept) -> Kept;
keep_bounded(_Rest, 0, Kept) -> Kept;
keep_bounded([F | Rest], K, Kept) -> keep_bounded(Rest, K - 1, [F | Kept]).

%% --- 16 KB strings ----------------------------------------------------

%% ~16 KB built by doubling from a ~128-byte seed: seven doublings.
%% Rebuilt every iteration so each lands as a fresh refcounted ProcBin
%% in the crash neighborhood. The seed carries multibyte UTF-8 (an
%% em-dash), matching the has-non-ASCII markers on the real gate
%% strings; content is otherwise inert.
big_string(I, Char) ->
    Seed = <<(tag(I, <<>>))/binary, Char,
             " thread 'main' panicked at src/lib.rs:1:1 ",
             226, 128, 148,
             " assertion failed: state-dependent, not content-dependent; "
             "the walk misreads the neighbour, not the string. ">>,
    double(Seed, 7).

double(B, 0) -> B;
double(B, K) -> double(<<B/binary, B/binary>>, K - 1).

%% Small varying prefix so no two iterations encode byte-identical
%% strings (defeats any interning/sharing shortcut).
tag(I, Rest) ->
    D0 = $0 + (I rem 10),
    D1 = $0 + ((I div 10) rem 10),
    D2 = $0 + ((I div 100) rem 10),
    <<D2, D1, D0, $:, Rest/binary>>.

%% --- the encode under test --------------------------------------------

%% Encode order mirrors dev_report_to_json: summary, then each claim's
%% criterion and how, then the gate strings, then per-run tails.
encode_report(Summary, Claims, {gate_outcome, _Pass, Runs, Diff, Diagnostics}) ->
    E0 = json:encode_binary(Summary),
    E1 = encode_claims(Claims),
    E2 = json:encode_binary(Diagnostics),
    E3 = json:encode_binary(Diff),
    E4 = encode_tails(Runs),
    [E0, E1, E2, E3, E4].

encode_claims([]) -> [];
encode_claims([{claim, Criterion, How} | Rest]) ->
    [{json:encode_binary(Criterion), json:encode_binary(How)}
     | encode_claims(Rest)].

encode_tails([]) -> [];
encode_tails([{run, Name, _Code, Tail} | Rest]) ->
    [{json:encode_binary(Name), json:encode_binary(Tail)}
     | encode_tails(Rest)].

%% Keep results live past the encode so nothing is collected early.
consume([_ | _]) -> ok;
consume([]) -> ok.
