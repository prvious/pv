package main

import (
	"fmt"

	"github.com/charmbracelet/huh"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/log"
)

func main() {
	style := lipgloss.NewStyle().Foreground(lipgloss.Color("205")).Bold(true)

	log.Info("Starting pv CLI")

	var projectName string
	form := huh.NewForm(
		huh.NewGroup(
			huh.NewInput().
				Title("Project name").
				Value(&projectName),
		),
	)

	if err := form.Run(); err != nil {
		log.Fatal("Error running form", "err", err)
	}

	fmt.Println(style.Render("Created project:"), projectName)
}