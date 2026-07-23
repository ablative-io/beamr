%% AION-ENCODE-GC-DEFECT sparse-trigger fixture: the badarg expression.
%%
%% Companion to encode_gc_repro.erl (the faithful bool-rich shape, whose
%% dense false-cons population expresses the 0.16.0 corruption as an
%% immediate SIGSEGV: a misread cons whose neighbour word is a SMALL raw
%% value gets that value dereferenced as an Arc pointer). This module
%% loads the dice the other way — the production expression: every
%% false-headed cons is allocated so its following allocation is a cons
%% whose first word is a BINARY POINTER (mapped memory). The misread
%% walk then survives its bogus Arc round-trip and writes zero over the
%% binary term slot; the later positional, unguarded encode — the
%% gleam_json_ffi shape, trusting the structure it built — hands the
%% zeroed term to json:encode_binary and takes badarg on content that
%% was valid when constructed. No other boolean cons cells exist in this
%% module, so the gentle path is the only one the walk can take.
-module(encode_gc_repro_badarg).
-export([main/0, main/1]).

main() -> main(50000).

main(N) ->
    loop(0, N).

loop(N, N) ->
    {completed, N};
loop(I, N) ->
    Pairs = pairs(I, 16),
    churn(I, 24),
    Encoded = encode_pairs(Pairs, I),
    consume(Encoded),
    loop(I + 1, N).

%% [Bin, Flag] pairs, bin FIRST in list order: right-to-left
%% construction allocates cons(false, Tail) and THEN cons(BinPtr, _) —
%% so each false-headed cons is immediately followed in allocation
%% order by a pointer-headed cons, the survivable misread shape.
pairs(_I, 0) -> [];
pairs(I, K) ->
    Bin = tag(I + K,
        <<"Resolved the repeated mechanical gate failure without changing "
          "the brief diff: the runner has the CI-pinned toolchain on its "
          "PATH and the raw battery passes in order, pure ASCII payload. ">>),
    Flag = I rem 2 =:= 2,
    [[Bin, Flag] | pairs(I, K - 1)].

%% Heap churn with NO boolean cells: integer lists and tuples only,
%% enough allocation to drive minor collections while Pairs is live.
churn(_I, 0) -> ok;
churn(I, K) ->
    consume(int_list(I + K, 40)),
    churn(I, K - 1).

int_list(_Seed, 0) -> [];
int_list(Seed, K) -> [{Seed + K, Seed * K} | int_list(Seed, K - 1)].

%% Positional and unguarded, like dev_report_to_json: the binary is
%% where the builder put it, no re-validation before the BIF call.
encode_pairs([], _I) -> [];
encode_pairs([[Bin, _Flag] | Rest], I) ->
    [json:encode_binary(Bin) | encode_pairs(Rest, I)].

tag(I, Rest) ->
    D0 = $0 + (I rem 10),
    D1 = $0 + ((I div 10) rem 10),
    D2 = $0 + ((I div 100) rem 10),
    <<D2, D1, D0, $:, Rest/binary>>.

consume([_ | _]) -> ok;
consume([]) -> ok;
consume({_, _}) -> ok.
