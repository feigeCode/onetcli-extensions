package protocol

import (
	"encoding/binary"
	"errors"
	"io"
)

var ErrMalformedLengthEncodedInteger = errors.New("malformed length-encoded integer")

func PutLengthEncodedInt(dst []byte, n uint64) []byte {
	switch {
	case n < 251:
		return append(dst, byte(n))
	case n < 1<<16:
		dst = append(dst, 0xfc)
		return binary.LittleEndian.AppendUint16(dst, uint16(n))
	case n < 1<<24:
		return append(dst, 0xfd, byte(n), byte(n>>8), byte(n>>16))
	default:
		dst = append(dst, 0xfe,
			byte(n), byte(n>>8), byte(n>>16), byte(n>>24),
			byte(n>>32), byte(n>>40), byte(n>>48), byte(n>>56),
		)
		return dst
	}
}

func PutLengthEncodedString(dst []byte, s string) []byte {
	dst = PutLengthEncodedInt(dst, uint64(len(s)))
	return append(dst, s...)
}

func ReadLengthEncodedInt(src []byte) (value uint64, consumed int, isNull bool, err error) {
	if len(src) == 0 {
		return 0, 0, false, io.ErrUnexpectedEOF
	}

	switch src[0] {
	case 0xfb:
		return 0, 1, true, nil
	case 0xfc:
		if len(src) < 3 {
			return 0, 0, false, io.ErrUnexpectedEOF
		}
		return uint64(binary.LittleEndian.Uint16(src[1:3])), 3, false, nil
	case 0xfd:
		if len(src) < 4 {
			return 0, 0, false, io.ErrUnexpectedEOF
		}
		return uint64(src[1]) | uint64(src[2])<<8 | uint64(src[3])<<16, 4, false, nil
	case 0xfe:
		if len(src) < 9 {
			return 0, 0, false, io.ErrUnexpectedEOF
		}
		return binary.LittleEndian.Uint64(src[1:9]), 9, false, nil
	case 0xff:
		return 0, 0, false, ErrMalformedLengthEncodedInteger
	default:
		return uint64(src[0]), 1, false, nil
	}
}

func ReadLengthEncodedString(src []byte) (value []byte, consumed int, isNull bool, err error) {
	n, used, isNull, err := ReadLengthEncodedInt(src)
	if err != nil || isNull {
		return nil, used, isNull, err
	}
	if uint64(len(src)-used) < n {
		return nil, 0, false, io.ErrUnexpectedEOF
	}
	end := used + int(n)
	return src[used:end], end, false, nil
}
