// cgo_cc wraps clang for Windows MSVC-target builds. Go's runtime/cgo adds
// -mthreads for Windows; clang rejects that MinGW-only flag for arm64 MSVC.
package main

import (
	"fmt"
	"os"
	"os/exec"
)

func filterArgs(args []string, target string) []string {
	if target != "aarch64-pc-windows-msvc" {
		return args
	}
	filtered := make([]string, 0, len(args))
	for _, arg := range args {
		if arg != "-mthreads" {
			filtered = append(filtered, arg)
		}
	}
	return filtered
}

func main() {
	target := os.Getenv("PROXYZMS_CGO_TARGET")
	if target == "" {
		fmt.Fprintln(os.Stderr, "PROXYZMS_CGO_TARGET is not set")
		os.Exit(2)
	}
	args := append([]string{"--target=" + target}, filterArgs(os.Args[1:], target)...)
	cmd := exec.Command("clang", args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			os.Exit(exitErr.ExitCode())
		}
		fmt.Fprintln(os.Stderr, "run clang:", err)
		os.Exit(1)
	}
}
