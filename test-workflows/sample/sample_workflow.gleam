/// Sample Gleam workflow for testing beamr-meridian NIF wiring.
///
/// Exercises the core NIF surface: read_file, run_cmd, write_file.
///
/// Compile: place this file and meridian_ffi.gleam in a gleam project's src/,
/// run `gleam build --target erlang`, and copy
/// build/dev/erlang/<project>/ebin/sample_workflow.beam beside this source.
/// (Last compiled with gleam 1.17.0; the committed gleam_stdlib beams are
/// original and are NOT regenerated on fixture changes.)

import gleam/result
import gleam/string
import meridian_ffi

pub type WorkflowResult {
  WorkflowResult(
    file_content: String,
    cmd_output: String,
    written: Bool,
  )
}

pub type WorkflowError {
  ReadFailed(reason: String)
  CmdFailed(reason: String)
  WriteFailed(reason: String)
}

/// Entry point — read a file, run a command, write output.
///
/// The output path is caller-supplied so concurrent test runs never share a
/// destination: a fixed path here means two `cargo test` invocations on the
/// same host race each other's cleanup.
pub fn run(
  input_path: String,
  output_path: String,
) -> Result(WorkflowResult, WorkflowError) {
  use content <- result.try(
    meridian_ffi.read_file(input_path)
    |> result.map_error(ReadFailed)
  )

  use _cmd_result <- result.try(
    meridian_ffi.run_cmd("echo 'hello from gleam'")
    |> result.map_error(CmdFailed)
  )

  let output = string.concat([
    "Input: ", content, "\nCmd ran successfully",
  ])

  use _ <- result.try(
    meridian_ffi.write_file(output_path, output)
    |> result.map_error(WriteFailed)
  )

  Ok(WorkflowResult(
    file_content: content,
    cmd_output: "hello from gleam",
    written: True,
  ))
}
