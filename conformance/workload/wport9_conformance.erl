%% WPORT-9 conformance workload — one exported entry per acceptance-shape
%% surface, executed FROM BYTECODE through the real generated bundle by
%% conformance/driver.mjs. Every entry terminates in a map the driver
%% machine-checks (or, for process_error/0, a deliberate abnormal exit).
%%
%% Surface discipline (brief WPORT-9 R2): only BIFs reachable under the
%% cooperative wasm registration set (gate1 + gate2 + stdlib stubs +
%% capability BIFs). No stdlib imports. Wake classes unreachable from
%% bytecode (trapped exit, native completion — no link facility, no JS
%% surface for native actors) are ledger rows with their T1 walls named,
%% not entries here.
-module(wport9_conformance).
%% Classic `catch` on purpose: it compiles to the Catch/CatchEnd
%% instructions the interpreter provably executes (the T1 refusal wall's
%% own mechanism); `try` would gamble on try_case support.
-compile(nowarn_deprecated_catch).
-export([
    wake_send/0,
    wake_send_child/1,
    wake_cast/0,
    wake_receive_timeout/0,
    wake_timer_deadline/0,
    capability_fetch/1,
    capability_kv/2,
    bif_supported/0,
    bif_unsupported/0,
    output_entry/0,
    process_error/0,
    armed_hold/0
]).

%% Mailbox-send wake + spawn edge: this process spawns a child (plain
%% spawn/3 — the one cooperative spawn shape), hands it self(), and parks
%% in receive; the child's `!` is the wake.
wake_send() ->
    Child = erlang:spawn(wport9_conformance, wake_send_child, [self()]),
    receive
        {wport9_child, Child, Value} ->
            exit(#{<<"entry">> => <<"wake_send">>,
                   <<"child_value">> => Value,
                   <<"spawned">> => true})
    end.

wake_send_child(Parent) ->
    Parent ! {wport9_child, self(), 42}.

%% Cast wake: parks in receive at true idle; the driver casts a
%% codec-native map (JS objects marshal to maps — tuples are not
%% JS-mintable).
wake_cast() ->
    receive
        #{<<"cast">> := Payload} ->
            exit(#{<<"entry">> => <<"wake_cast">>,
                   <<"payload">> => Payload})
    end.

%% Receive-timeout wake: nothing arrives; the after-clause fires.
wake_receive_timeout() ->
    receive
        {never, _} -> exit(#{<<"entry">> => <<"wake_receive_timeout">>,
                             <<"outcome">> => <<"unexpected_message">>})
    after 40 ->
        exit(#{<<"entry">> => <<"wake_receive_timeout">>,
               <<"outcome">> => <<"timed_out">>})
    end.

%% Timer-deadline wake from bytecode: send_after arms the unified Deliver
%% wheel; the delivery is the wake. cancel_timer on a fresh timer proves
%% the cancel path returns an integer (remaining ms) rather than false.
wake_timer_deadline() ->
    Cancelled = erlang:send_after(60000, self(), wport9_never),
    Remaining = erlang:cancel_timer(Cancelled),
    erlang:send_after(40, self(), wport9_tick),
    receive
        wport9_tick ->
            exit(#{<<"entry">> => <<"wake_timer_deadline">>,
                   <<"tick">> => true,
                   <<"cancelled_had_remaining">> => Remaining > 0})
    end.

%% Async-NIF / Promise-completion wake, fetch arm: suspend on the
%% capability op; the promise settlement is the wake.
capability_fetch(Url) ->
    {ok, Response} = wasm_fetch:request(#{<<"url">> => Url}),
    exit(#{<<"entry">> => <<"capability_fetch">>,
           <<"response">> => Response}).

%% Async-NIF / Promise-completion wake, KV arm: put/get round-trip,
%% delete idempotence, lexicographic listing.
capability_kv(Key, Value) ->
    {ok, true} = wasm_kv:put(Key, Value),
    {ok, Stored} = wasm_kv:get(Key),
    {ok, Keys} = wasm_kv:list_by_prefix(<<"wport9:">>),
    {ok, true} = wasm_kv:delete(Key),
    {ok, undefined} = wasm_kv:get(Key),
    exit(#{<<"entry">> => <<"capability_kv">>,
           <<"stored">> => Stored,
           <<"keys">> => Keys,
           <<"deleted">> => true}).

%% Supported-BIF entry: maps construction/fold, term comparison, self/0 —
%% the profile's supported core exercised as values.
bif_supported() ->
    Map = maps:from_list([{<<"a">>, 1}, {<<"b">>, 2}, {<<"c">>, 3}]),
    Sum = maps:fold(fun(_K, V, Acc) -> Acc + V end, 0, Map),
    Ordered = <<"a">> < <<"b">>,
    SelfIsPid = is_pid(self()),
    exit(#{<<"entry">> => <<"bif_supported">>,
           <<"sum">> => Sum,
           <<"ordered">> => Ordered,
           <<"self_is_pid">> => SelfIsPid,
           <<"keys">> => maps:keys(Map)}).

%% Unsupported-BIF entry: statistics/1 refuses badarg (no system_info
%% facility — the deliberate WPORT-5 refusal); the catch shape IS the
%% typed refusal value observable from bytecode.
bif_unsupported() ->
    Caught = (catch erlang:statistics(runtime)),
    Refused = case Caught of
        {'EXIT', {badarg, _}} -> <<"badarg_caught">>;
        _ -> <<"unexpected">>
    end,
    exit(#{<<"entry">> => <<"bif_unsupported">>,
           <<"refusal">> => Refused}).

%% Output entry: two ordered sink writes; the driver asserts both arrive
%% through the registered sink callback in order.
output_entry() ->
    ok = io:put_chars(<<"wport9 output line one\n">>),
    ok = io:put_chars(<<"wport9 output line two\n">>),
    exit(#{<<"entry">> => <<"output_entry">>,
           <<"wrote">> => 2}).

%% Process-error entry: deliberate abnormal exit with a recognizable
%% reason; the driver asserts the typed error surfaces via the JS-visible
%% exit-error surface, not just a dead pid.
process_error() ->
    erlang:exit(wport9_deliberate_error).

%% Armed-future-deadline hold: parks with a far-future receive timer so
%% the F-0d window can observe that an armed deadline produces ONE
%% arming one-shot and then silence — never recurring callbacks. The
%% process is discarded with the VM; it never exits during the run.
armed_hold() ->
    receive
        wport9_release ->
            exit(#{<<"entry">> => <<"armed_hold">>,
                   <<"outcome">> => <<"released">>})
    after 600000 ->
        exit(#{<<"entry">> => <<"armed_hold">>,
               <<"outcome">> => <<"expired">>})
    end.
