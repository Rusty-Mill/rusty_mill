// An HTTP/2 (h2c, prior knowledge) server on Go's own net/http, the
// implementation a real control server speaks. ts-control's client tests
// drive its HTTP/2 session against it. Prints "LISTENING <addr>".
package main

import (
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
)

func main() {
	mux := http.NewServeMux()
	// Echoes the protocol, Host and body length as JSON.
	mux.HandleFunc("/machine/register", func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		w.Header().Set("content-type", "application/json")
		fmt.Fprintf(w, `{"proto":%q,"host":%q,"len":%d}`, r.Proto, r.Host, len(body))
	})
	// Streams three netmap-style frames: a little-endian u32 length, then JSON.
	mux.HandleFunc("/machine/map", func(w http.ResponseWriter, r *http.Request) {
		io.ReadAll(r.Body)
		flusher := w.(http.Flusher)
		for i := 0; i < 3; i++ {
			msg := []byte(fmt.Sprintf(`{"frame":%d}`, i))
			var header [4]byte
			binary.LittleEndian.PutUint32(header[:], uint32(len(msg)))
			w.Write(header[:])
			w.Write(msg)
			flusher.Flush()
		}
	})
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("LISTENING %s\n", ln.Addr())
	protocols := new(http.Protocols)
	protocols.SetUnencryptedHTTP2(true)
	srv := &http.Server{Handler: mux, Protocols: protocols}
	if err := srv.Serve(ln); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
