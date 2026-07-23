%% WPORT-8 sitting handler — the real-bundle BEAM side of
%% WPORT-8-PROBE-CAPABILITY.md. One exported entry per protocol leg; the
%% sitting page picks the entry and passes sitting-controlled URLs/keys as
%% spawn args (JSON strings arrive as binaries). No stdlib calls: the
%% module exercises exactly the wasm_fetch/wasm_kv capability surface plus
%% pattern matching, so every observation is the adapters' own behavior.
-module(wport8_probe).
-export([leg1/4, leg2_reject/1, leg2_cancel/1, leg2_refusal/1, leg3/1]).

%% Leg 1 (the A4/rider-2 worker-shaped end-to-end): one real fetch AND a
%% KV round trip (put -> get -> list_by_prefix), the exit value assembled
%% from BOTH results. Failure on any step is a badmatch exit — honest, the
%% leg proves the happy path.
leg1(FetchUrl, KvKey, KvValue, KvPrefix) ->
    {ok, Response} = wasm_fetch:request(#{<<"url">> => FetchUrl}),
    {ok, true} = wasm_kv:put(KvKey, KvValue),
    {ok, Stored} = wasm_kv:get(KvKey),
    {ok, Keys} = wasm_kv:list_by_prefix(KvPrefix),
    #{<<"fetch_response">> => Response,
      <<"kv_stored_value">> => Stored,
      <<"kv_keys_under_prefix">> => Keys}.

%% Leg 2, rejection: the page points this at a refused/unroutable target;
%% the exit value is the typed error tuple verbatim —
%% {error, {rejected, DetailBinary}} with the browser's real failure text.
leg2_reject(Url) ->
    wasm_fetch:request(#{<<"url">> => Url}).

%% Leg 2, cancellation (host-abort arm): the page points this at the slow
%% endpoint and fires the recorded AbortController mid-flight; the exit
%% value is {error, {cancelled, DetailBinary}} through the normal seam.
leg2_cancel(Url) ->
    wasm_fetch:request(#{<<"url">> => Url}).

%% Leg 2, refusal: spawned on a VM with NO kv capability registered — the
%% synchronous typed refusal {error, {capability_missing, kv}} is the exit
%% value; no suspend, no host turn.
leg2_refusal(Key) ->
    wasm_kv:get(Key).

%% Leg 3 (NO-POLLING under real completion): one request against the slow
%% endpoint while the page's timer shims watch the in-flight window; the
%% exit value is the D8 response map of the eventual settle.
leg3(Url) ->
    {ok, Response} = wasm_fetch:request(#{<<"url">> => Url}),
    Response.
