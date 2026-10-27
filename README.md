# cortex9-mcp-diffuzz

[Español](#español) · [English](#english)

## Español

Fuzzer **diferencial y con estado** para servidores MCP (Model Context Protocol) sobre **stdio**, escrito en Rust.
Genera secuencias de mensajes (ciclo de vida, `tools/*`, `ping`, JSON roto, ids duplicados o inválidos, líneas enormes, batches, mensajes fuera de orden),
las envía a dos o más implementaciones, normaliza las respuestas, detecta divergencias, cuelgues, caídas y violaciones de JSON-RPC 2.0,
y reduce cada hallazgo a una reproducción mínima (delta debugging) que queda guardada como caso de regresión.

Targets de esta versión:

| nombre | implementación | cómo se construye |
|---|---|---|
| `mcp-server-rs` | [federiconbaez/mcp-server-rs](https://github.com/federiconbaez/mcp-server-rs) en el commit `9da13ddb13a4f33870873ce1fe8d995a3819f1a8` (sin cambios) | `scripts/build-targets.sh` lo clona y lo compila |
| `rmcp` | `targets/rmcp-echo`: servidor mínimo sobre el SDK oficial `rmcp = "=3.8.0"`, con una sola tool `echo` que tiene el mismo contrato que la de mcp-server-rs en lo que genera el fuzzer (rmcp-echo no declara `maxLength` ni `additionalProperties: false`, así que el generador no manda textos de más de 10000 caracteres ni propiedades extra) | parte de este workspace |

Alcance: sólo el subconjunto ciclo de vida + `tools/list` + `tools/call` + `ping`, y sólo transporte stdio.

### Cómo funciona

- `step`: vocabulario de 27 tipos de mensaje (válidos, malformados y fuera de orden). Los casos se guardan en forma simbólica, no como bytes.
- `gen`: generador con estado y semilla. El 75 % de los casos arranca con `initialize` (versión al azar entre 2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28 y una inexistente) y el 80 % de esos manda `notifications/initialized`. Los ids a veces se reutilizan a propósito.
- `target`: un proceso hijo por target, una línea por paso. Espera la respuesta hasta un timeout (2 s más 100 ms por MiB). Después de una notificación escucha 150 ms. Al final cierra stdin y espera la salida (si no sale en 3 s, lo mata).
- `norm`: borra lo que puede variar legítimamente (mensajes de error, nombre del servidor, capacidades, instrucciones, tools extra, orden de claves y de batches, texto de errores de tool y la decoración de 2026-07-28: `resultType`, `_meta`, `ttlMs`, `cacheScope`). Conserva si hay respuesta, si es resultado o error, el código de error, la versión negociada, si `echo` aparece en la lista, el texto devuelto, `isError` y la forma del `id` (igual al pedido, `null` o ausente).
- `oracle`: (1) **divergencia**, cuando las observaciones normalizadas difieren; (2) **cuelgue**, cuando un request no recibe respuesta; (3) **caída**, cuando el proceso sale con stdin abierto o no sale limpio con EOF; (4) **invariantes JSON-RPC 2.0**, por ejemplo respuesta sin `id`, `jsonrpc` distinto de `"2.0"`, `id` cambiado, más de una respuesta o respuesta a una notificación. Después de un cuelgue o una caída deja de comparar ese caso, porque las respuestas podrían quedar mal atribuidas.
- `minimize`: ddmin (Zeller y Hildebrandt) sobre la secuencia de pasos, con el predicado "se sigue reproduciendo la misma firma". La firma no incluye ids, posiciones, contenido de batches ni el texto devuelto.
- `corpus`: deduplica por firma, minimiza y guarda los casos en JSON. `replay` y el test `regressions` vuelven a correrlos.

### Uso

```sh
eval "$(scripts/build-targets.sh)"            # compila ambos targets y exporta las rutas
target/release/mcp-diffuzz fuzz \
  --target "mcp-server-rs=$DIFFUZZ_MCP_SERVER_RS" --target "rmcp=$DIFFUZZ_RMCP" \
  --seed 1 --cases 300 --out out/
target/release/mcp-diffuzz replay \
  --target "mcp-server-rs=$DIFFUZZ_MCP_SERVER_RS" --target "rmcp=$DIFFUZZ_RMCP" regressions/*.json
cargo test -p mcp-diffuzz --test regressions  # falla (no se saltea) si faltan los binarios
scripts/coverage.sh 300 1                     # cobertura: conformidad oficial vs diffuzz
```

Cualquier servidor stdio sirve como target: `--target nombre=comando args...`.

### Resultados medidos

Campaña de referencia: `mcp-diffuzz fuzz ... --seed 1 --cases 300 --out out/`, corrida en la máquina de desarrollo (Linux x86_64).

| casos | pasos | hallazgos crudos | firmas únicas | divergencias | caídas | cuelgues | invariantes | tiempo |
|---|---|---|---|---|---|---|---|---|
| 300 | 2022 | 1034 | **66** | 39 | 6 | 5 | 16 | 502 s |

Se corrió dos veces con la misma semilla y dio los mismos conteos.

Esos 66 casos minimizados están en `regressions/`, y el job `regressions` del CI vuelve a correrlos contra los dos targets en cada push.
Las firmas se agrupan en estas causas raíz (la agrupación es manual; cada fila apunta a casos concretos):

| id | tipo | target | descripción | evidencia | firmas | pasos mín. | ejemplo |
|---|---|---|---|---|---|---|---|
| R1 | caída | rmcp | rmcp termina el proceso (exit 1) si el primer mensaje no es `initialize`: una notificación (`initialized`, `cancelled`, desconocida, `initialize` sin id) o una respuesta del cliente | leído en el código: `ServerInitializeError::ExpectedInitializeRequest` | 10 | 1 | `regressions/crash-cancelled-f82dec09.json` |
| R2 | cuelgue | rmcp | rmcp ignora en silencio una línea que no es JSON válido (sin -32700, `id: null`); el cliente espera para siempre | leído en el código: es intencional (comentario que cita rust-sdk#938) | 8 | 1 | `regressions/divergence-malformed-0-9bba2b0e.json` |
| R3 | cuelgue | rmcp | rmcp descarta en silencio una línea de más de 16 MiB sin responder; mcp-server-rs no tiene límite y la responde | leído en el código: `DEFAULT_MAX_LINE_LENGTH` = 16 MiB | 2 | 1 | `regressions/divergence-oversized-8a12549b.json` |
| R4 | invariante | rmcp | rmcp responde errores sin el miembro `id` (JSON-RPC 2.0 §5: REQUIRED, `null` si no se pudo detectar) | leído en el código: `TxJsonRpcMessage::error(..., None)` | 27 | 1 | `regressions/divergence-batch-dfacf4dc.json` |
| R5 | divergencia | rmcp | rmcp rechaza batches aun con la versión 2025-03-26 negociada (esa revisión los admite); mcp-server-rs los procesa |  | 1 | 2 | `regressions/divergence-batch-935d8651.json` |
| R6 | invariante | ambos | se responde con error a un batch hecho sólo de notificaciones (JSON-RPC 2.0 §4.1 y §6: no se responde nada). rmcp siempre; mcp-server-rs cuando no se negoció 2025-03-26 (dudoso: desde 2025-06-18 MCP ya no tiene batches) |  | 3 | 1 | `regressions/divergence-batch-of-notifications-5ae0879b.json` |
| R7 | divergencia | rmcp | rmcp acepta un segundo `initialize` en la misma sesión y renegocia la versión; mcp-server-rs lo rechaza con -32600 |  | 3 | 2 | `regressions/divergence-initialize-7083a8fe.json` |
| R8 | divergencia | rmcp | con estado: tras un segundo `initialize` que pide 2026-07-28, rmcp contesta `2025-11-25` pero pasa a comportarse como 2026-07-28 (`ping` da -32601 y `tools/list` sale con decoración stateless). Rompe "respetar la versión negociada" (MCP Lifecycle) |  | 1 | 3 | `regressions/divergence-ping-5ad6becd.json` |
| R9 | divergencia | rmcp | con estado: después de un request stateless de 2026-07-28 como primer mensaje, un `ping` sin `_meta` da -32602 en rmcp y `{}` en mcp-server-rs |  | 1 | 2 | `regressions/divergence-ping-69ea9660.json` |
| R10 | divergencia | rmcp | un `initialize` sin params después de inicializar da -32601 (method not found) en rmcp y -32600 en mcp-server-rs |  | 1 | 2 | `regressions/divergence-initialize-no-params-7c9373da.json` |
| R11 | caída | rmcp | rmcp sale con código 1 si stdin se cierra (o sólo llega basura) antes de `initialize` |  | 1 | 1 | `regressions/crash-eof-unclean-exit-556bec95.json` |
| C | divergencia | - | códigos de error distintos donde la spec no fija uno: requests antes de `initialize` (-32600 en mcp-server-rs, -32602 en rmcp) y `params` como array (-32602 en mcp-server-rs, -32600 en rmcp). No se cuentan como bugs |  | 8 | 1 | `regressions/divergence-call-echo-641c8ea6.json` |

**Criterio de aceptación (≥3 divergencias, caídas o cuelgues con repro mínima):** se cumple. Hay 2 causas de caída (R1, R11), 2 de cuelgue (R2, R3), 1 de invariante (R4) y 6 de divergencia (R5 a R10), todas con repro de 1 a 3 pasos guardada en `regressions/`. Las de la fila C no se cuentan.

**Sobre mcp-server-rs:** no se modificó ese repo; sus problemas sólo se reportan acá. Los dos huecos documentados se probaron así:
- **Sin límite de línea en stdio:** es real y aparece como divergencia en `oversized` (mcp-server-rs responde a una línea de 16 MiB, rmcp la descarta).
- **Ids duplicados no detectados:** el fuzzer los genera, pero no produce divergencia, porque los dos servidores responden a cada request por separado. Con stdio secuencial no hay dos requests en vuelo con el mismo id, así que este diferencial no puede mostrarlo como bug.

### Cobertura frente a la suite oficial de conformidad

Comando: `scripts/coverage.sh 300 1`. Compila mcp-server-rs con `-C instrument-coverage` y corre tres cargas: (a) la suite oficial `@modelcontextprotocol/conformance@0.1.16` (`server --url`, suite `active`, 30 escenarios) contra su transporte HTTP; (b) mcp-diffuzz con `--seed 1 --cases 300 --no-minimize` contra su transporte stdio; (c) la unión de las dos. Se mide cobertura de **líneas** y de **regiones** con `llvm-cov`. La de **ramas** no se midió, porque necesita nightly (`-Z coverage-options=branch`).

| file | conformance lines | diffuzz lines | union lines | conformance regions | diffuzz regions | union regions |
|---|---|---|---|---|---|---|
| calc.rs | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% |
| jsonrpc.rs | 62.3% | 84.4% | 84.4% | 75.7% | 88.1% | 88.1% |
| main.rs | 0.0% | 21.7% | 21.7% | 0.0% | 22.7% | 22.7% |
| protocol.rs | 27.3% | 27.3% | 27.3% | 27.8% | 27.8% | 27.8% |
| sandbox.rs | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% |
| server.rs | 48.2% | 63.1% | 75.0% | 46.1% | 63.0% | 75.8% |
| tools.rs | 35.7% | 66.1% | 66.1% | 26.8% | 63.6% | 63.6% |
| transport/http.rs | 63.7% | 0.0% | 63.7% | 60.5% | 0.0% | 60.5% |
| transport/stdio.rs | 0.0% | 97.1% | 97.1% | 0.0% | 91.2% | 91.2% |
| **core (jsonrpc+server+protocol+tools)** | 48.4% | 67.5% | 74.4% | 47.5% | 67.4% | 74.7% |
| **all files** | 40.4% | 34.8% | 55.9% | 39.1% | 34.6% | 55.2% |

Cómo leer la tabla:
- En el núcleo independiente del transporte (`jsonrpc.rs`, `server.rs`, `protocol.rs`, `tools.rs`), diffuzz solo cubre 67.5 % de líneas contra 48.4 % de la suite oficial (+19.1 puntos). Sumado a la suite llega a 74.4 % (+26.0 puntos sobre la suite sola). La meta de la idea era +20 puntos de cobertura de ramas, pero lo que se midió son líneas y regiones, no ramas.
- En el total de archivos diffuzz solo queda **por debajo** de la suite (34.8 % contra 40.4 %), porque no toca el transporte HTTP. La comparación por archivo es la honesta.
- La suite oficial dio `7 passed, 24 failed` contra mcp-server-rs. La mayoría de las fallas se deben a que espera tools, resources y prompts de su servidor de prueba (`test_simple_text`, etc.) que mcp-server-rs no tiene. Igual es la línea base que pide la consigna: la suite oficial tal cual contra el mismo binario.
- `calc.rs` y `sandbox.rs` quedan en 0 % con las dos cargas: el diferencial sólo usa `echo`, porque rmcp-echo no tiene `calc`.

### Qué está probado y qué no

Probado:
- Tests unitarios del generador, la normalización, los oráculos, ddmin, el corpus y el runner (con procesos `sh` de juguete que hacen eco, se cuelgan, se caen o ignoran EOF).
- El test de regresión reproduce cada caso guardado contra los binarios reales. Si faltan las variables `DIFFUZZ_MCP_SERVER_RS` o `DIFFUZZ_RMCP`, **falla**; no se saltea.
- Las causas raíz de rmcp marcadas como "leído en el código" se confirmaron leyendo el código de `rmcp 3.8.0` (`src/transport/async_rw.rs`, `src/service/server.rs`).

No probado o fuera de alcance:
- Transporte HTTP streamable, OAuth, sampling y elicitation, y clientes MCP.
- Una gramática derivada automáticamente del schema: el vocabulario está escrito a mano a partir de la spec 2025-11-25.
- Los otros SDKs oficiales (TypeScript, Python, etc.). Se pueden usar como targets stdio, pero no se corrieron.
- Campañas largas: la meta original de "24 h de CPU" no se corrió; la referencia es de 300 casos.
- Fallos dependientes del tiempo: la detección de cuelgues usa timeouts, así que una máquina muy cargada podría dar falsos "NO-REPLY".
- Las divergencias de código de error (por ejemplo -32600 contra -32602 antes de `initialize`) no son necesariamente bugs, porque la spec no fija el código. Se listan aparte.
- No se abrieron issues en los proyectos afectados.

### Fuentes leídas

- Especificación JSON-RPC 2.0: https://www.jsonrpc.org/specification (secciones 4.1, 5 y 6).
- MCP 2025-11-25, Lifecycle: https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
- README del framework oficial de conformidad: https://github.com/modelcontextprotocol/conformance
- README del SDK oficial de Rust: https://github.com/modelcontextprotocol/rust-sdk
- Código de `rmcp 3.8.0` (crates.io), `src/transport/async_rw.rs` y `src/service/server.rs`.
- Código y README de mcp-server-rs en el commit fijado.

### Licencia

Apache-2.0. Ver [LICENSE](LICENSE).

---

## English

A **stateful, differential** fuzzer for MCP (Model Context Protocol) servers over **stdio**, written in Rust.
It generates message sequences (lifecycle, `tools/*`, `ping`, broken JSON, duplicate or invalid ids, huge lines, batches, out-of-order messages),
sends them to two or more implementations, normalizes the replies, detects divergences, hangs, crashes and JSON-RPC 2.0 violations,
and shrinks every finding to a minimal reproduction (delta debugging) that is stored as a regression case.

Targets in this version:

| name | implementation | how it is built |
|---|---|---|
| `mcp-server-rs` | [federiconbaez/mcp-server-rs](https://github.com/federiconbaez/mcp-server-rs) at commit `9da13ddb13a4f33870873ce1fe8d995a3819f1a8` (unmodified) | `scripts/build-targets.sh` clones and builds it |
| `rmcp` | `targets/rmcp-echo`: minimal server on the official SDK `rmcp = "=3.8.0"`, with a single `echo` tool that has the same contract as mcp-server-rs's for what the fuzzer generates (rmcp-echo declares neither `maxLength` nor `additionalProperties: false`, so the generator sends no text over 10000 characters and no extra properties) | part of this workspace |

Scope: only the lifecycle + `tools/list` + `tools/call` + `ping` subset, and only the stdio transport.

### How it works

- `step`: a vocabulary of 27 message kinds (valid, malformed and out-of-order). Cases are stored symbolically, not as bytes.
- `gen`: a seeded, stateful generator. 75 % of cases start with `initialize` (random version among 2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28 and a bogus one), and 80 % of those send `notifications/initialized`. Ids are sometimes reused on purpose.
- `target`: one child process per target, one line per step. It waits for the reply up to a timeout (2 s plus 100 ms per MiB) and listens for 150 ms after a notification. At the end it closes stdin and waits for exit (killing the process after 3 s).
- `norm`: erases what may legitimately differ (error messages, server name, capabilities, instructions, extra tools, key and batch order, tool error texts, and the 2026-07-28 decoration: `resultType`, `_meta`, `ttlMs`, `cacheScope`). It keeps whether there is a reply, result vs. error, the error code, the negotiated version, whether `echo` is listed, the echoed text, `isError`, and the shape of the `id` (same as the request, `null`, or absent).
- `oracle`: (1) **divergence**, when normalized observations differ; (2) **hang**, when a request gets no reply; (3) **crash**, when the process exits while stdin is open or does not exit cleanly on EOF; (4) **JSON-RPC 2.0 invariants**, such as a reply without `id`, `jsonrpc` other than `"2.0"`, a changed `id`, more than one reply, or a reply to a notification. After a hang or a crash it stops comparing that case, since replies could be misattributed.
- `minimize`: ddmin (Zeller and Hildebrandt) over the step sequence, with the predicate "the same signature still reproduces". Signatures leave out ids, positions, batch contents and echoed text.
- `corpus`: deduplicates by signature, minimizes, and saves cases as JSON. `replay` and the `regressions` test run them again.

### Usage

```sh
eval "$(scripts/build-targets.sh)"            # builds both targets and exports their paths
target/release/mcp-diffuzz fuzz \
  --target "mcp-server-rs=$DIFFUZZ_MCP_SERVER_RS" --target "rmcp=$DIFFUZZ_RMCP" \
  --seed 1 --cases 300 --out out/
target/release/mcp-diffuzz replay \
  --target "mcp-server-rs=$DIFFUZZ_MCP_SERVER_RS" --target "rmcp=$DIFFUZZ_RMCP" regressions/*.json
cargo test -p mcp-diffuzz --test regressions  # fails (is not skipped) if the binaries are missing
scripts/coverage.sh 300 1                     # coverage: official conformance vs diffuzz
```

Any stdio server can be a target: `--target name=command args...`.

### Measured results

Reference campaign: `mcp-diffuzz fuzz ... --seed 1 --cases 300 --out out/`, run on the development machine (Linux x86_64).

| cases | steps | raw findings | unique signatures | divergences | crashes | hangs | invariants | time |
|---|---|---|---|---|---|---|---|---|
| 300 | 2022 | 1034 | **66** | 39 | 6 | 5 | 16 | 502 s |

It was run twice with the same seed and gave the same counts.

Those 66 minimized cases are in `regressions/`, and the CI `regressions` job replays them against both targets on every push.
The signatures group into these root causes (the grouping is manual; each row points to concrete cases):

| id | kind | target | description | evidence | signatures | min. steps | example |
|---|---|---|---|---|---|---|---|
| R1 | crash | rmcp | rmcp exits (status 1) when the first message is not `initialize`: a notification (`initialized`, `cancelled`, unknown, id-less `initialize`) or a client response | read in the code: `ServerInitializeError::ExpectedInitializeRequest` | 10 | 1 | `regressions/crash-cancelled-f82dec09.json` |
| R2 | hang | rmcp | rmcp silently ignores a line that is not valid JSON (no -32700, `id: null`); the client waits forever | read in the code: intentional (comment citing rust-sdk#938) | 8 | 1 | `regressions/divergence-malformed-0-9bba2b0e.json` |
| R3 | hang | rmcp | rmcp silently drops a line longer than 16 MiB without replying; mcp-server-rs has no limit and answers it | read in the code: `DEFAULT_MAX_LINE_LENGTH = 16 MiB` | 2 | 1 | `regressions/divergence-oversized-8a12549b.json` |
| R4 | invariant | rmcp | rmcp sends error replies without the `id` member (JSON-RPC 2.0 §5: REQUIRED, `null` if it could not be detected) | read in the code: `TxJsonRpcMessage::error(..., None)` | 27 | 1 | `regressions/divergence-batch-dfacf4dc.json` |
| R5 | divergence | rmcp | rmcp rejects batches even after negotiating 2025-03-26 (which allows them); mcp-server-rs processes them |  | 1 | 2 | `regressions/divergence-batch-935d8651.json` |
| R6 | invariant | both | an error is sent for a batch made only of notifications (JSON-RPC 2.0 §4.1 and §6: nothing must be returned). rmcp always; mcp-server-rs when 2025-03-26 was not negotiated (borderline: MCP dropped batches in 2025-06-18) |  | 3 | 1 | `regressions/divergence-batch-of-notifications-5ae0879b.json` |
| R7 | divergence | rmcp | rmcp accepts a second `initialize` in the same session and renegotiates the version; mcp-server-rs rejects it with -32600 |  | 3 | 2 | `regressions/divergence-initialize-7083a8fe.json` |
| R8 | divergence | rmcp | stateful: after a second `initialize` asking for 2026-07-28, rmcp answers `2025-11-25` but starts behaving as 2026-07-28 (`ping` returns -32601 and `tools/list` carries stateless decoration). Breaks "respect the negotiated protocol version" (MCP Lifecycle) |  | 1 | 3 | `regressions/divergence-ping-5ad6becd.json` |
| R9 | divergence | rmcp | stateful: after a stateless 2026-07-28 request as the first message, a `ping` without `_meta` gets -32602 from rmcp and `{}` from mcp-server-rs |  | 1 | 2 | `regressions/divergence-ping-69ea9660.json` |
| R10 | divergence | rmcp | an `initialize` without params after initialization gets -32601 (method not found) from rmcp and -32600 from mcp-server-rs |  | 1 | 2 | `regressions/divergence-initialize-no-params-7c9373da.json` |
| R11 | crash | rmcp | rmcp exits with status 1 if stdin closes (or only garbage arrives) before `initialize` |  | 1 | 1 | `regressions/crash-eof-unclean-exit-556bec95.json` |
| C | divergence | - | different error codes where the spec does not fix one: requests before `initialize` (-32600 in mcp-server-rs, -32602 in rmcp) and `params` as an array (-32602 in mcp-server-rs, -32600 in rmcp). Not counted as bugs |  | 8 | 1 | `regressions/divergence-call-echo-641c8ea6.json` |

**Acceptance criterion (≥3 divergences, crashes or hangs with a minimal repro):** met. There are 2 crash causes (R1, R11), 2 hang causes (R2, R3), 1 invariant cause (R4) and 6 divergence causes (R5 to R10), each with a 1-3 step repro stored in `regressions/`. Row C is not counted.

**About mcp-server-rs:** that repo was not modified; its problems are only reported here. Its two documented gaps were tested as follows:
- **No stdio line-length limit:** real, and it shows up as the `oversized` divergence (mcp-server-rs answers a 16 MiB line, rmcp drops it).
- **Duplicate ids not detected:** the fuzzer generates them, but they cause no divergence, because both servers answer each request separately. With sequential stdio there are never two in-flight requests with the same id, so this differential cannot surface it as a bug.

### Coverage against the official conformance suite

Command: `scripts/coverage.sh 300 1`. It builds mcp-server-rs with `-C instrument-coverage` and runs three workloads: (a) the official suite `@modelcontextprotocol/conformance@0.1.16` (`server --url`, `active` suite, 30 scenarios) against its HTTP transport; (b) mcp-diffuzz with `--seed 1 --cases 300 --no-minimize` against its stdio transport; (c) the union of both. **Line** and **region** coverage come from `llvm-cov`. **Branch** coverage was not measured, because it needs nightly (`-Z coverage-options=branch`).

| file | conformance lines | diffuzz lines | union lines | conformance regions | diffuzz regions | union regions |
|---|---|---|---|---|---|---|
| calc.rs | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% |
| jsonrpc.rs | 62.3% | 84.4% | 84.4% | 75.7% | 88.1% | 88.1% |
| main.rs | 0.0% | 21.7% | 21.7% | 0.0% | 22.7% | 22.7% |
| protocol.rs | 27.3% | 27.3% | 27.3% | 27.8% | 27.8% | 27.8% |
| sandbox.rs | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% |
| server.rs | 48.2% | 63.1% | 75.0% | 46.1% | 63.0% | 75.8% |
| tools.rs | 35.7% | 66.1% | 66.1% | 26.8% | 63.6% | 63.6% |
| transport/http.rs | 63.7% | 0.0% | 63.7% | 60.5% | 0.0% | 60.5% |
| transport/stdio.rs | 0.0% | 97.1% | 97.1% | 0.0% | 91.2% | 91.2% |
| **core (jsonrpc+server+protocol+tools)** | 48.4% | 67.5% | 74.4% | 47.5% | 67.4% | 74.7% |
| **all files** | 40.4% | 34.8% | 55.9% | 39.1% | 34.6% | 55.2% |

How to read the table:
- On the transport-independent core (`jsonrpc.rs`, `server.rs`, `protocol.rs`, `tools.rs`), diffuzz alone covers 67.5 % of lines vs 48.4 % for the official suite (+19.1 points). Together with the suite it reaches 74.4 % (+26.0 points over the suite alone). The idea's goal was +20 points of branch coverage, but what was measured is lines and regions, not branches.
- Over all files, diffuzz alone is **below** the suite (34.8 % vs 40.4 %), because it never touches the HTTP transport. The per-file comparison is the honest one.
- The official suite reported `7 passed, 24 failed` against mcp-server-rs. Most failures come from it expecting its own test server's tools, resources and prompts (`test_simple_text`, etc.), which mcp-server-rs does not have. It is still the baseline the task asked for: the official suite, as is, against the same binary.
- `calc.rs` and `sandbox.rs` stay at 0 % under both workloads: the differential only uses `echo`, because rmcp-echo has no `calc`.

### What is tested and what is not

Tested:
- Unit tests for the generator, normalization, oracles, ddmin, the corpus and the runner (using toy `sh` processes that echo, hang, crash or ignore EOF).
- The regression test reproduces every stored case against the real binaries. If `DIFFUZZ_MCP_SERVER_RS` or `DIFFUZZ_RMCP` is missing, it **fails**; it is not skipped.
- The rmcp root causes marked "read in the code" were confirmed by reading the `rmcp 3.8.0` source (`src/transport/async_rw.rs`, `src/service/server.rs`).

Not tested, or out of scope:
- Streamable HTTP, OAuth, sampling and elicitation, and MCP clients.
- A grammar derived automatically from the schema: the vocabulary is handwritten from the 2025-11-25 spec.
- The other official SDKs (TypeScript, Python, etc.). They can be used as stdio targets, but were not run.
- Long campaigns: the original "24 CPU hours" goal was not run; the reference is 300 cases.
- Timing-dependent failures: hang detection uses timeouts, so a heavily loaded machine could produce false "NO-REPLY" results.
- Error-code divergences (for example -32600 vs -32602 before `initialize`) are not necessarily bugs, since the spec does not fix the code. They are listed separately.
- No issues were filed with the affected projects.

### Sources read

- JSON-RPC 2.0 specification: https://www.jsonrpc.org/specification (sections 4.1, 5 and 6).
- MCP 2025-11-25, Lifecycle: https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
- Official conformance framework README: https://github.com/modelcontextprotocol/conformance
- Official Rust SDK README: https://github.com/modelcontextprotocol/rust-sdk
- `rmcp 3.8.0` source (crates.io), `src/transport/async_rw.rs` and `src/service/server.rs`.
- mcp-server-rs source and README at the pinned commit.

### License

Apache-2.0. See [LICENSE](LICENSE).
