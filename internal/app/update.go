package app

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/log"
)

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height

	case tea.KeyMsg:
		switch msg.String() {
		case "ctrl+c", "esc":
			return m, tea.Quit

		case "up":
			if m.selected > 0 {
				m.selected--
			}

		case "down":
			if m.selected < len(m.filteredOpts)-1 {
				m.selected++
			}

		case "enter":
			if len(m.filteredOpts) > 0 {
				chosen := m.filteredOpts[m.selected]
				log.Info("Selected option", "choice", chosen)

				if action, exists := GetActions()[chosen]; exists {
					if err := action(); err != nil {
						log.Error("Action failed", "error", err)
					} else {
						log.Info("Action completed successfully")
					}
				}

				return m, tea.Quit
			}

		case "backspace":
			if len(m.searchInput) > 0 {
				m.searchInput = m.searchInput[:len(m.searchInput)-1]
				m.filterOptions()
			}

		case "ctrl+backspace", "alt+backspace":
			if len(m.searchInput) > 0 {
				words := strings.Fields(m.searchInput)
				if len(words) > 1 {
					m.searchInput = strings.Join(words[:len(words)-1], " ") + " "
				} else {
					m.searchInput = ""
				}
				m.filterOptions()
			}

		default:
			if len(msg.String()) == 1 {
				m.searchInput += msg.String()
				m.filterOptions()
			}
		}
	}

	return m, nil
}

func (m *Model) filterOptions() {
	if m.searchInput == "" {
		m.filteredOpts = m.options
	} else {
		m.filteredOpts = []string{}
		query := strings.ToLower(m.searchInput)
		for _, option := range m.options {
			if strings.Contains(strings.ToLower(option), query) {
				m.filteredOpts = append(m.filteredOpts, option)
			}
		}
	}

	if m.selected >= len(m.filteredOpts) {
		m.selected = len(m.filteredOpts) - 1
	}
	if m.selected < 0 {
		m.selected = 0
	}
}
