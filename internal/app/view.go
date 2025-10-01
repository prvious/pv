package app

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

func (m Model) View() string {
	var s strings.Builder

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("205")).
		PaddingLeft(2).
		PaddingRight(2)

	searchStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("62")).
		Padding(0, 1).
		Width(50)

	selectedStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("205")).
		Bold(true).
		PaddingLeft(2)

	normalStyle := lipgloss.NewStyle().
		PaddingLeft(4)

	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("241")).
		PaddingTop(1)

	if m.height > 0 {
		s.WriteString("\n")
	}

	s.WriteString(titleStyle.Render("🪄 PV CLI - Local Dev Environment Manager"))
	s.WriteString("\n\n")

	searchBox := fmt.Sprintf("Search: %s", m.searchInput)
	if len(m.searchInput) == 0 {
		searchBox = "Search: (type to filter options)"
	}
	s.WriteString(searchStyle.Render(searchBox))
	s.WriteString("\n\n")

	visibleStart := 0
	visibleEnd := len(m.filteredOpts)
	maxVisible := m.height - 10

	if maxVisible > 0 && len(m.filteredOpts) > maxVisible {
		if m.selected >= maxVisible/2 {
			visibleStart = m.selected - maxVisible/2
			visibleStart = min(visibleStart, len(m.filteredOpts)-maxVisible)
		}
		visibleEnd = visibleStart + maxVisible
		visibleEnd = min(visibleEnd, len(m.filteredOpts))
	}

	for i := visibleStart; i < visibleEnd; i++ {
		cursor := "  "
		style := normalStyle

		if i == m.selected {
			cursor = ">"
			style = selectedStyle
		}

		s.WriteString(style.Render(fmt.Sprintf("%s %s", cursor, m.filteredOpts[i])))
		s.WriteString("\n")
	}

	if len(m.filteredOpts) == 0 {
		s.WriteString(normalStyle.Render("  No options match your search..."))
		s.WriteString("\n")
	}

	s.WriteString("\n")
	s.WriteString(helpStyle.Render("  ↑/↓: navigate • enter: select • backspace: delete char • ctrl+w: delete word • ctrl+c/esc: quit"))

	return s.String()
}
