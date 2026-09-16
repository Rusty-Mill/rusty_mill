;; Review finding #9 regression: `host::read_file` must route through
;; `ForgePathValidator::validate` (the same validator primitive
;; `host::write_file` uses via `validate_for_write`) instead of a bare
;; `canonicalize()`-then-`read()` pair. Requests a path that climbs above
;; the forge root ("../escape.txt") and asserts the call is denied with
;; the same `HOST_CAPABILITY_DENIED` code `deny_path_traversal` returns
;; for both read and write.
;;
;; The data segment lays out the literal "../escape.txt" (13 bytes) at
;; offset 0, reserving 100..4196 as a scratch output buffer.

(module
  ;; host::read_file(path_ptr, path_len, out_ptr, out_cap) -> i32
  (import "host" "read_file"
    (func $host_read_file (param i32 i32 i32 i32) (result i32)))

  (memory (export "memory") 1)

  ;; Place "../escape.txt" (13 bytes, no trailing NUL) at offset 0.
  (data (i32.const 0) "../escape.txt")

  (func (export "probe") (result i32)
    ;; read_file(path_ptr=0, path_len=13, out_ptr=100, out_cap=4096)
    i32.const 0
    i32.const 13
    i32.const 100
    i32.const 4096
    call $host_read_file)
)
