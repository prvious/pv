//go:build windows

package installer

import (
	"errors"
	"os"
	"syscall"
	"unsafe"
)

const (
	moveFileReplaceExisting = 0x1
	moveFileWriteThrough    = 0x8
)

var moveFileExW = syscall.NewLazyDLL("kernel32.dll").NewProc("MoveFileExW")

func replaceFile(tempPath string, path string) error {
	from, err := syscall.UTF16PtrFromString(tempPath)
	if err != nil {
		return err
	}
	to, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		return err
	}

	result, _, err := moveFileExW.Call(
		uintptr(unsafe.Pointer(from)),
		uintptr(unsafe.Pointer(to)),
		uintptr(moveFileReplaceExisting|moveFileWriteThrough),
	)
	if result != 0 {
		return nil
	}
	if err != syscall.Errno(0) {
		return os.NewSyscallError("MoveFileExW", err)
	}
	return os.NewSyscallError("MoveFileExW", errors.New("file replacement failed"))
}
