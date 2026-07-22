%% End-to-end monitor/2 -> receive {'DOWN', Ref, ...} match fixture.
%%
%% Headline evidence for the boxed-reference landing (the exact hot path this
%% unblocks: gleam_otp actor.call). `watch/1` monitors an already-dead Target,
%% so the supervision layer delivers an immediate DOWN, then selective-receives
%% on the BOUND reference returned by monitor/2. It returns `matched` only when
%% monitor/2's return term is term-equal to the reference the DOWN message
%% carries; otherwise it falls through to the bounded after-branch and returns
%% `no_match`. Before the boxed-reference fix, monitor/2 returned a small int
%% while the DOWN carried a boxed reference — different term ranks that never
%% match — so this probe returned `no_match`.
-module(monitor_down_probe).
-export([watch/1, target/0]).

%% A target that exits normally the moment it is run. The test runs it to
%% completion first, so `watch/1` monitors an already-dead pid.
target() ->
    ok.

watch(Target) ->
    Ref = erlang:monitor(process, Target),
    receive
        {'DOWN', Ref, process, _Object, _Reason} ->
            matched
    after 3000 ->
        no_match
    end.
