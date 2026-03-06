package ui

import (
	"fmt"
	"os"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/lipgloss/table"
)

// TreeItem represents one node in the tree with a title and detail line.
type TreeItem struct {
	Title  string
	Detail string
}

// Tree renders a list of items as a tree with box-drawing characters.
func Tree(items []TreeItem) {
	for i, item := range items {
		isLast := i == len(items)-1

		branch := Purple.Render("├─")
		cont := Purple.Render("│")
		if isLast {
			branch = Purple.Render("└─")
			cont = " "
		}

		fmt.Fprintf(os.Stderr, "  %s %s\n", branch, item.Title)
		fmt.Fprintf(os.Stderr, "  %s  %s\n", cont, Muted.Render(item.Detail))

		if !isLast {
			fmt.Fprintf(os.Stderr, "  %s\n", cont)
		}
	}
}

var (
	purple   = lipgloss.ANSIColor(141)
	gray     = lipgloss.ANSIColor(245)
	lightGray = lipgloss.ANSIColor(241)

	headerStyle  = lipgloss.NewStyle().Foreground(purple).Bold(true)
	cellStyle    = lipgloss.NewStyle().Padding(0, 1)
	oddRowStyle  = cellStyle.Foreground(gray)
	evenRowStyle = cellStyle.Foreground(lightGray)
)

// Table renders a styled lipgloss table.
func Table(headers []string, rows [][]string) {
	t := table.New().
		Border(lipgloss.RoundedBorder()).
		BorderStyle(lipgloss.NewStyle().Foreground(purple)).
		StyleFunc(func(row, col int) lipgloss.Style {
			switch {
			case row == table.HeaderRow:
				return headerStyle
			case row%2 == 0:
				return evenRowStyle
			default:
				return oddRowStyle
			}
		}).
		Headers(headers...).
		Rows(rows...)

	rendered := t.String()
	for _, line := range strings.Split(rendered, "\n") {
		fmt.Fprintf(os.Stderr, "  %s\n", line)
	}
}
