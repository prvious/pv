package app

import (
	"os"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/log"
)

func Run() {
	log.Info("Starting pv CLI")

	p := tea.NewProgram(InitialModel(), tea.WithAltScreen())
	if _, err := p.Run(); err != nil {
		log.Fatal("Error running program", "err", err)
		os.Exit(1)
	}
}
