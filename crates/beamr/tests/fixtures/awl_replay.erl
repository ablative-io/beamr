%%% BEAMR-R8-DEOPT R4 — a WORKFLOW-SHAPED execution whose routing step takes the
%%% error edge, for replay parity across the deopt boundary.
%%%
%%% DERIVED FROM `workflows/gates/gates.awl` in our own public first-party repo
%%% `ablative-io/aion` — the seven-leg lane battery, its `GateResult` records,
%%% and the `step manifest` routing decision:
%%%
%%%   gates.awl:135-152  the band sequence — first_band (fmt / bun /
%%%                      clippy_workspace), clippy_server_band, test_band,
%%%                      embed_band, gates_worker_band
%%%   gates.awl:65-88    `type GateResult { gate, exit_code, passed, ... }`
%%%   gates.awl:153-158  `step manifest` — `outcome green: when <every leg
%%%                      passed> and receipt.legs_recorded == 7, route clean` /
%%%                      `outcome red: otherwise, route failing`
%%%
%%% WHAT WAS CUT. The actions, the workspace plumbing, the failure_detail and
%%% output_tail prose, and the manifest derivation are all cut; what is kept is
%%% the SHAPE — steps accumulate leg results, a receipt is derived from them, and
%%% a routing step selects an outcome from a token. As in the R3 specimens, the
%%% `otherwise` arm of the routing select is cut so the structurally-cold
%%% fall-through trap AWL emits for every select (`emit/control.rs:133-134`) is
%%% actually TAKEN. The whole run's output term is what R4 compares, not one
%%% function's error: `{run, Legs, Token, Outcome}` carries every leg result, the
%%% derived token, and the routed-or-trapped outcome including the full
%%% class/reason/stacktrace.

-module(awl_replay).

-export([run/1, route/1]).

run(Mode) ->
    Legs = legs(Mode),
    Token = token(Mode),
    Outcome =
        try route(Token) of
            Routed -> {routed, Routed}
        catch
            Class:Reason:Stack -> {trapped, Class, Reason, Stack}
        end,
    {run, Legs, Token, Outcome}.

%% gates.awl:135-152 — the seven legs, each carrying its command's REAL exit
%% code as data. `failing` reds the test leg, as a real battery does.
legs(clean) ->
    [leg(fmt, 0), leg(bun, 0), leg(clippy_workspace, 0), leg(clippy_server, 0),
     leg(tests, 0), leg(embed, 0), leg(gates_worker, 0)];
legs(failing) ->
    [leg(fmt, 0), leg(bun, 0), leg(clippy_workspace, 0), leg(clippy_server, 0),
     leg(tests, 101), leg(embed, 0), leg(gates_worker, 0)];
legs(unmeasured) ->
    [leg(fmt, 0), leg(bun, 0), leg(clippy_workspace, 0), leg(clippy_server, 0),
     leg(tests, 0), leg(embed, 0), leg(gates_worker, 0)].

%% gates.awl:65-70 — `passed` is true exactly when the command exited 0. Written
%% as two clauses rather than `ExitCode =:= 0` so the fixture needs no BIF
%% registry and the run is self-contained.
leg(Gate, 0) -> {gate_result, Gate, 0, true};
leg(Gate, ExitCode) -> {gate_result, Gate, ExitCode, false}.

%% The derived routing token. `unmeasured` is the token the production document
%% never produces, and it is what drives this run onto the cold trap edge.
token(clean) -> green;
token(failing) -> red;
token(unmeasured) -> no_verdict.

%% gates.awl:153-158 — `step manifest`'s routing select, `otherwise` arm cut.
%% Written as ONE clause with an inner `case` on purpose: a two-clause
%% `route(green) -> ...; route(red) -> ...` traps through `func_info` (the LEG 1c
%% A2 landing pad, which was already Supported) instead of through `CaseEnd`, and
%% R4 must cross THIS brief's terminal. erlc emits select_val + a fall-through
%% `{case_end,{x,0}}`; this is the function put under JIT, and `no_verdict` is
%% what takes its cold edge.
route(Token) ->
    case Token of
        green -> clean;
        red -> failing
    end.
