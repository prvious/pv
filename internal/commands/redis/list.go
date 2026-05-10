package redis

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "redis:list",
	GroupID: "redis",
	Short:   "Show Redis status (single row)",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed.")
			return nil
		}

		st, _ := r.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		precise := "?"
		if vs != nil {
			if v := vs.Get("redis"); v != "" {
				precise = v
			}
		}

		runState := "stopped"
		if status != nil {
			if s, ok := status.Supervised["redis"]; ok && s.Running {
				runState = "running"
			}
		}
		wanted := st.Wanted
		if wanted == "" {
			wanted = "—"
		}

		projects := []string{}
		if reg != nil {
			for _, p := range reg.List() {
				if p.Services != nil && p.Services.Redis {
					projects = append(projects, p.Name)
				}
			}
		}
		projectsCol := "—"
		if len(projects) > 0 {
			projectsCol = strings.Join(projects, ",")
		}

		rows := [][]string{{
			precise,
			fmt.Sprintf("%d", r.PortFor()),
			fmt.Sprintf("%s (%s)", runState, wanted),
			config.RedisDataDir(),
			projectsCol,
		}}
		ui.Table([]string{"VERSION", "PORT", "STATUS", "DATA DIR", "LINKED PROJECTS"}, rows)
		return nil
	},
}
