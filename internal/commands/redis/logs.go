package redis

import (
	"io"
	"os"
	"os/exec"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var logsFollow bool

var logsCmd = &cobra.Command{
	Use:     "redis:logs",
	GroupID: "redis",
	Short:   "Tail the Redis log file",
	Long:    "Reads ~/.pv/logs/redis.log. With -f / --follow, tails the file like `tail -f`.",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		path := config.RedisLogPath()
		if logsFollow {
			c := exec.Command("tail", "-f", path)
			c.Stdout = os.Stdout
			c.Stderr = os.Stderr
			return c.Run()
		}
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		defer f.Close()
		_, err = io.Copy(os.Stdout, f)
		return err
	},
}

func init() {
	logsCmd.Flags().BoolVarP(&logsFollow, "follow", "f", false, "Follow the log (tail -f)")
}
