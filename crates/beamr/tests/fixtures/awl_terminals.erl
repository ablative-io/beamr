%%% BEAMR-R8-DEOPT — the four error-raising terminals, lowered as DEOPT
%%% terminals. These are the R3 error-IDENTITY specimens and the R3b null arm.
%%%
%%% DERIVED FROM THE CENSUS CORPUS, not written as synthetic minimums (R3b).
%%% Every specimen below is cut down from a production document in our own
%%% public, first-party repo `ablative-io/aion` — the same documents the AWL
%%% variant census names as poisoned:
%%%
%%%   verdict_code/1         <- workflows/investigate/investigate.awl:148
%%%                             `type Verdict = Reproduced | NotReproduced
%%%                              | AlreadyFixed | NeedsInfo | WrongRepo`
%%%   first_failing_target/1 <- workflows/gates/gates.awl:73
%%%                             `failing_targets: [String]` on `type GateResult`
%%%   report_presence/1      <- workflows/investigate/investigate.awl:273
%%%                             `outcome present: when probe_result.report is
%%%                              present, route file_it`
%%%   gate_passed/1          <- workflows/gates/gates.awl:65-88
%%%                             `type GateResult { gate, exit_code, passed,
%%%                              failing_targets, failure_detail, output_tail }`
%%%
%%% WHAT WAS CUT, AND WHY THE CUT IS LOAD-BEARING. In AWL the fall-through trap
%%% these compile to is emitted STRUCTURALLY and is cold by construction:
%%% `crates/aion-awl/src/mir/select/emit/control.rs:133-134` pushes the fail
%%% label and then `CaseEnd` for EVERY select, total arms or not, and
%%% `emit/burst.rs:293-295` / `:385-395` do the same for every `let assert`.
%%% That is exactly why these traps poison 51 of 9,864 corpus functions while
%%% never firing in production. To WITNESS the taken edge — R3's acceptance —
%%% the specimen must drop the total-coverage arm (the `outcome ... otherwise`
%%% clause) so the structurally-cold edge is actually TAKEN. The prose docs, the
%%% String payloads and the action plumbing are cut as irrelevant to the
%%% terminal; the routed values are kept as the small ints / atoms AWL routes on.

-module(awl_terminals).

-export([verdict_code/1, first_failing_target/1, report_presence/1,
         gate_passed/1,
         probe_verdict/1, probe_failing_target/1, probe_presence/1,
         probe_gate_passed/1,
         happy_verdict/1, happy_failing_target/1, happy_presence/1,
         happy_gate_passed/1,
         subject/1, tail_at_failure/1]).

%% `type GateResult` (gates.awl:65-88), as the record whose field read traps.
-record(gate_result, {gate, exit_code, passed, failing_targets,
                      failure_detail, output_tail}).

%% ---------------------------------------------------------------------------
%% THE SPECIMENS. Each is the function put under JIT: its whole body is
%% Supported and its only cold edge is the error terminal under test.
%% ---------------------------------------------------------------------------

%% CaseEnd — investigate.awl's five-way Verdict dispatch, `otherwise` arm cut.
%% erlc emits select_val + a fall-through `{case_end,{x,0}}`, the same shape
%% AWL's `emit/control.rs:133-134` emits for every select.
verdict_code(V) ->
    case V of
        reproduced -> 1;
        not_reproduced -> 2;
        already_fixed -> 3;
        needs_info -> 4;
        wrong_repo -> 5
    end.

%% Badmatch — gates.awl's `failing_targets: [String]` read through the AWL
%% `let assert` burst shape. THE SUBJECT/TAIL DISTINCTION IS THE POINT: erlc
%% walks the tail into {x,2} (get_list/get_tl) and leaves {x,0} holding the
%% WHOLE SUBJECT, so `{badmatch,{x,0}}` reports the subject — the same invariant
%% AWL reaches by re-pointing X0 from its Y home before trapping
%% (burst.rs:385-395, the BC-2b-5 carried fix). A five-element subject against a
%% three-element pattern fails at `is_nil` with a NON-EMPTY PROPER SUFFIX still
%% in {x,2}, so subject and tail are distinguishable terms and the fixture's
%% assertion cannot be vacuous.
first_failing_target(FailingTargets) ->
    [First, _Second, _Third] = FailingTargets,
    First.

%% IfEnd — investigate.awl's `when probe_result.report is present` presence
%% guard, `otherwise` arm cut. BEAM's reason is the BARE ATOM `if_clause`.
report_presence(Report) ->
    if
        Report =/= absent -> present
    end.

%% Badrecord — a `GateResult` field read against a value that is not one.
gate_passed(R) ->
    R#gate_result.passed.

%% ---------------------------------------------------------------------------
%% Argument construction, in bytecode. The Rust driver passes an ATOM SELECTOR
%% and the fixture builds the real corpus-shaped argument itself, so the driver
%% never has to synthesise heap terms outside a process.
%% ---------------------------------------------------------------------------

%% gates.awl's failing-target list: cargo's own rerun invocations, one per
%% failing target. Five entries, so the three-element assert fails at `is_nil`
%% with `[embed_target, bun_target]` still in the walked-tail register.
targets(full) ->
    [fmt_target, clippy_workspace_target, test_target, embed_target, bun_target];
targets(exact) ->
    [fmt_target, clippy_workspace_target, test_target].

%% The subject the trap must report, and the walked tail it must NOT report.
%% Returned as data so the fixture's own assertion is written against terms
%% this module produced, never against a string typed into the test.
subject(full) -> targets(full).
tail_at_failure(full) -> [embed_target, bun_target].

gate_result(ok) ->
    #gate_result{gate = fmt, exit_code = 0, passed = true,
                 failing_targets = [], failure_detail = none,
                 output_tail = none};
gate_result(bad) ->
    not_a_gate_result.

%% ---------------------------------------------------------------------------
%% CATCHERS. Interpreted by construction — `build_stacktrace` is wave-2 and
%% stays RejectedIncremental, so the pre-pass declines these, which is exactly
%% what we want: the SPECIMEN is the function under JIT and the catcher merely
%% observes. Each returns the FULL error identity as data: class, reason term,
%% and the raw stacktrace term.
%% ---------------------------------------------------------------------------

probe_verdict(V) ->
    try verdict_code(V) of
        Result -> {ok, Result}
    catch
        Class:Reason:Stack -> {caught, Class, Reason, Stack}
    end.

probe_failing_target(Sel) ->
    try first_failing_target(targets(Sel)) of
        Result -> {ok, Result}
    catch
        Class:Reason:Stack -> {caught, Class, Reason, Stack}
    end.

probe_presence(Report) ->
    try report_presence(Report) of
        Result -> {ok, Result}
    catch
        Class:Reason:Stack -> {caught, Class, Reason, Stack}
    end.

%% The record is built BEFORE the try, so the call to `gate_passed/1` inside it
%% is a plain `call` and not a `call_last`. That matters: `call_last` does not
%% consult the JIT cache at all (interpreter/opcodes/core.rs `call_last` pops the
%% frame and jumps), so a specimen only ever reached by a tail call from a
%% frame-bearing caller is NEVER entered natively — recorded as finding F8.
probe_gate_passed(Sel) ->
    R = gate_result(Sel),
    try gate_passed(R) of
        Result -> {ok, Result}
    catch
        Class:Reason:Stack -> {caught, Class, Reason, Stack}
    end.

%% ---------------------------------------------------------------------------
%% R3b NULL ARM — the HAPPY path of the same poisoned-then-cured specimens.
%% These never reach a trap. They exist so the cure can be shown not to perturb
%% the path that never traps, measured in the SAME run as the deopt arm, and
%% they are also the heat that drives the specimen into the JIT cache.
%% ---------------------------------------------------------------------------

happy_verdict(V) -> verdict_code(V).
happy_failing_target(Sel) -> first_failing_target(targets(Sel)).
happy_presence(Report) -> report_presence(Report).
%% Two clauses, each building its record inline, so erlc emits `call_only` to
%% `gate_passed/1` rather than the `call_last` that never dispatches (F8).
happy_gate_passed(ok) ->
    gate_passed(#gate_result{gate = fmt, exit_code = 0, passed = true,
                             failing_targets = [], failure_detail = none,
                             output_tail = none});
happy_gate_passed(bad) ->
    gate_passed(not_a_gate_result).
