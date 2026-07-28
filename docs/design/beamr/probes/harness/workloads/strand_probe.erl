%% WPORT-7 PROBE-FAILURE section 2 workload — the timer-strand confirm-or-kill
%% rider (arc :92). Fresh-authored, OTP 29 erlc.
%%
%% On `go`, arms a 0ms self-timer IN THE SAME TURN (mid-turn, after the wheel
%% cursor has advanced within the drain) then parks for its delivery. The page
%% measures wall-clock between send_message(go) and await_exit resolution: a
%% cluster near a full wheel revolution (~1s) with re-arm churn CONFIRMS the
%% strand; macrotask-scale delivery with one arm -> one fire KILLS it.
-module(strand_probe).
-export([run/0]).

%% The outer receive matches ANY message: the host trigger arrives via
%% send_message, which marshals a JS string to a BINARY (no atom encoding on
%% that boundary — see convert.rs / README), so a bare-atom clause would never
%% match. The inner `probe` is a real atom delivered by send_after and matches.
run() ->
    receive
        _Go ->
            erlang:send_after(0, self(), probe),
            receive
                probe -> got_probe
            end
    end.
