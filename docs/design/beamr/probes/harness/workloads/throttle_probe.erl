%% WPORT-3 PROBE-THROTTLE workload (fresh-authored, OTP 29 erlc).
%%
%% Parks a VM with BOTH deadline classes pending so the unified host deadline
%% service arms exactly one setTimeout at the earliest (the T+30s receive-after)
%% and the T+45s native Deliver rides behind it.
-module(throttle_probe).
-export([wait30/0, deliver45/0, park/0, arm/1]).

%% Receive-after deadline class: requested at T+30s, exits `timed_out` on fire.
wait30() ->
    receive
    after 30000 -> timed_out
    end.

%% Native Deliver deadline class: self-armed `erlang:send_after/3` at T+45s
%% targeting this same parked receive process, exits `got_it` on delivery.
%% Self-target is deliberate: a pid cannot cross the JS `spawn(module, fn,
%% args_json)` boundary (a pid term marshals to a plain integer there — see
%% convert.rs and README), so the armer and the target must be one process.
deliver45() ->
    erlang:send_after(45000, self(), deliver),
    receive
        deliver -> got_it
    end.

%% Cross-boundary pid variant, DOCUMENTED-LIMITED (not used by the page):
%% park for `deliver`, exit `got_it`. Pairs with arm/1.
park() ->
    receive
        deliver -> got_it
    end.

%% arm(Target): `erlang:send_after(45000, Target, deliver)`. Requires a real
%% pid term for Target; the JS spawn-args boundary cannot deliver one, so this
%% pair is retained for documentation only. Use deliver45/0 instead.
arm(Target) ->
    erlang:send_after(45000, Target, deliver).
