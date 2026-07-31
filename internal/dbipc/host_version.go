package dbipc

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strconv"
)

const MinimumHostVersion = "0.10.0"

var semverPattern = regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$`)

type initParams struct {
	HostVersion string `json:"host_version"`
}

func ValidateHostVersion(raw json.RawMessage) error {
	var params initParams
	if len(raw) == 0 || string(raw) == "null" {
		return fmt.Errorf("this driver requires Navop >= %s; host_version is missing", MinimumHostVersion)
	}
	if err := json.Unmarshal(raw, &params); err != nil {
		return fmt.Errorf("invalid init params: %w", err)
	}
	major, minor, _, prerelease, ok := parseSemver(params.HostVersion)
	if !ok {
		return fmt.Errorf("this driver requires Navop >= %s; invalid host_version %q", MinimumHostVersion, params.HostVersion)
	}
	if prerelease != "" || (major == 0 && minor < 10) {
		return fmt.Errorf("this driver requires Navop >= %s; current host version is %s. Please upgrade Navop", MinimumHostVersion, params.HostVersion)
	}
	return nil
}

func parseSemver(value string) (major, minor, patch int, prerelease string, ok bool) {
	match := semverPattern.FindStringSubmatch(value)
	if match == nil {
		return 0, 0, 0, "", false
	}
	major64, err1 := strconv.ParseInt(match[1], 10, 32)
	minor64, err2 := strconv.ParseInt(match[2], 10, 32)
	patch64, err3 := strconv.ParseInt(match[3], 10, 32)
	if err1 != nil || err2 != nil || err3 != nil {
		return 0, 0, 0, "", false
	}
	return int(major64), int(minor64), int(patch64), match[4], true
}
