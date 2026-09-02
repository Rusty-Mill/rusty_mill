// Interop test server: a Noise responder built on the REAL Go
// tailscale.com/control/controlbase package, so the Rust controlbase
// implementation can be verified byte-for-byte against ground truth.
//
// Usage: noise-server-go -listen 127.0.0.1:0
//
// Prints two lines to stdout, then serves:
//
//	CONTROL_KEY mkey:<hex>
//	LISTENING <addr>
//
// For each accepted connection it runs the controlbase server handshake
// (protocol version 123 expected) and then echoes every received byte
// back, prefixed once with "GO-OK <client mkey>\n".
package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"os"

	"tailscale.com/control/controlbase"
	"tailscale.com/types/key"
)

func main() {
	listen := flag.String("listen", "127.0.0.1:0", "address to listen on")
	flag.Parse()

	controlKey := key.NewMachine()
	ln, err := net.Listen("tcp", *listen)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("CONTROL_KEY %s\n", controlKey.Public().String())
	fmt.Printf("LISTENING %s\n", ln.Addr().String())
	os.Stdout.Sync()

	for {
		c, err := ln.Accept()
		if err != nil {
			log.Fatal(err)
		}
		go serve(c, controlKey)
	}
}

func serve(c net.Conn, controlKey key.MachinePrivate) {
	defer c.Close()
	conn, err := controlbase.Server(context.Background(), c, controlKey, nil)
	if err != nil {
		log.Printf("handshake failed: %v", err)
		return
	}
	if _, err := fmt.Fprintf(conn, "GO-OK %s\n", conn.Peer().String()); err != nil {
		log.Printf("write failed: %v", err)
		return
	}
	if _, err := io.Copy(conn, conn); err != nil && err != io.EOF {
		log.Printf("echo ended: %v", err)
	}
}
