%% Fixture pinning stacktrace-head PROVENANCE for a nested raise.
%%
%% `top_fun/0` first triggers-and-catches a `function_clause` (via `miss/1`),
%% which — on the pre-derivation `main` — leaves `Process::current_mfa` set to
%% the STALE failing MFA `{mfa_provenance, miss, 1}` (func_info is its only
%% writer). It then drives `f/1 -> g/1`, where `g/1` raises a `badmatch`, and
%% returns the FUNCTION atom of the stacktrace TOP entry.
%%
%% Pre-fix the head mis-attributes to the stale `miss`; after derive-at-read it
%% is the true raising function `g`. `id/1` launders every argument so erlc
%% cannot constant-fold the failing calls. Compile with `erlc mfa_provenance.erl`
%% (OTP 25+) and commit the .beam next to this source.
-module(mfa_provenance).
-export([top_fun/0]).

%% id/1 routes arguments at runtime so erlc cannot see the mismatches.
id(X) -> X.

top_fun() ->
    %% Reach func_info on a failed dispatch and swallow it: on `main` this
    %% leaves current_mfa pointing at miss/1 (the stale-provenance seed).
    try miss(id(bad)) catch _:_ -> ok end,
    try f(id(b))
    catch _Class:_Reason:Stack -> top_name(Stack)
    end.

%% The head frame's function atom — the provenance the stacktrace head reports.
top_name([{_Module, Function, _Arity, _Info} | _]) -> Function;
top_name(_) -> undefined.

f(X) -> g(X).

%% Raises error:{badmatch, b}: the true head function is g/1.
g(X) -> a = X.

%% Single literal clause: miss(bad) matches nothing and reaches func_info.
miss(a) -> ok.
