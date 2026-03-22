package types

import (
	"fmt"
	"strings"
)

// QueryString converts ListOptions into a URL query string (without leading "?").
// Returns an empty string if no options are set.
func (o *ListOptions) QueryString() string {
	if o == nil {
		return ""
	}
	var parts []string
	if o.Limit > 0 {
		parts = append(parts, fmt.Sprintf("limit=%d", o.Limit))
	}
	if o.Offset > 0 {
		parts = append(parts, fmt.Sprintf("offset=%d", o.Offset))
	}
	if o.Sort != "" {
		parts = append(parts, "sort="+o.Sort)
	}
	return strings.Join(parts, "&")
}

// AppendQuery appends ListOptions as query parameters to a URL path.
// If the path already contains '?', parameters are appended with '&'.
func AppendQuery(path string, opts *ListOptions) string {
	if opts == nil {
		return path
	}
	qs := opts.QueryString()
	if qs == "" {
		return path
	}
	if strings.Contains(path, "?") {
		return path + "&" + qs
	}
	return path + "?" + qs
}
