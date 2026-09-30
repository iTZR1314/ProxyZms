package main

import (
	"reflect"
	"testing"
)

func TestFilterArgsDropsMingwFlagOnlyForArm64Msvc(t *testing.T) {
	input := []string{"-O2", "-mthreads", "-c", "shim.c", "-o", "shim.o"}
	arm64 := filterArgs(input, "aarch64-pc-windows-msvc")
	wantArm64 := []string{"-O2", "-c", "shim.c", "-o", "shim.o"}
	if !reflect.DeepEqual(arm64, wantArm64) {
		t.Fatalf("arm64 args = %v, want %v", arm64, wantArm64)
	}
	if amd64 := filterArgs(input, "x86_64-pc-windows-msvc"); !reflect.DeepEqual(amd64, input) {
		t.Fatalf("amd64 args = %v, want unchanged %v", amd64, input)
	}
}
