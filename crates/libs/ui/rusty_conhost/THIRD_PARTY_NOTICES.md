# Third-Party Notices

## Windows Terminal (microsoft/terminal): layouts and behaviour

`rusty_conhost` is a Rust console host written for this repository. It is
not a binding, a vendored copy or a build of microsoft/terminal's C++
source. Parts of it are transcribed or ported from that source, which is
MIT-licensed, at commit `09beddc6`:

- `src/msg.rs`: ConDrv function codes, console API numbers, NTSTATUS
  values, the `CONSOLE_SERVER_MSG` layout and the message layouts
  (`dep/Console/condrv.h`, `conmsgl1.h`, `conmsgl2.h`,
  `src/server/ApiMessage.h`, `IoSorter.cpp`), and conhost's
  HRESULT-to-NTSTATUS mapping (`src/server/precomp.h`, `ApiSorter.cpp`).
- `src/console.rs`: connection, object and share-mode handling, console
  modes and their defaults, the title rules, and the GetConsoleLangId
  answer (`src/server/IoDispatchers.cpp`, `ApiDispatchers.cpp`,
  `ApiDispatchersInternal.cpp`, `ObjectHandle.cpp`, `ObjectHeader.cpp`,
  `src/host/getset.cpp`, `srvinit.cpp`, `inputBuffer.cpp`,
  `screenInfo.hpp`).

Those files carry the notice below at their top.

### microsoft/terminal's license

```
MIT License

Copyright (c) Microsoft Corporation. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
