## Examples of supported type of commands (basic for now):
> [!NOTE]
The commands below are just to test *Clanix* running commands,
not what *Clanix* is made to do, after the very first release, *Clanix* will
do much more, not merely run some basic commands.


### Very basic

- `clanix run mkdir docs`
```
Process ran for 1.045322ms while the timeout was 1ms
Clanix: mkdir docs failed!
```

- `clanix --dt run mkdir docs` // Disable the timeout
```
Clanix: mkdir docs was successful!
```

- `clanix --td --wd docs -v -s output.toml run ping 0.0.0.0 -c 5`
```
error: unexpected argument '--td' found

Usage: clanix [OPTIONS] <COMMAND>

For more information, try '--help'.
```

- `clanix --dt --wd docs -v -s output.toml run ping 0.0.0.0 -c 5` // It's --dt, not --td
```
error: unexpected argument '-c' found

  tip: to pass '-c' as a value, use '-- -c'

Usage: clanix run <PROGRAM> [ARGS]...

For more information, try '--help'.
```

- `clanix --dt --wd docs -v -s output.toml run ping 0.0.0.0 -- -c 5` // To run another command with `--`, you gotta add an extra `--` in front of it
```
Clanix: Command ping 0.0.0.0 -c 5 was ran successfully!
PING 0.0.0.0 (127.0.0.1) 56(84) bytes of data.
64 bytes from 127.0.0.1: icmp_seq=1 ttl=64 time=0.125 ms
64 bytes from 127.0.0.1: icmp_seq=2 ttl=64 time=0.104 ms
64 bytes from 127.0.0.1: icmp_seq=3 ttl=64 time=0.072 ms
64 bytes from 127.0.0.1: icmp_seq=4 ttl=64 time=0.103 ms
64 bytes from 127.0.0.1: icmp_seq=5 ttl=64 time=0.110 ms

--- 0.0.0.0 ping statistics ---
5 packets transmitted, 5 received, 0% packet loss, time 4103ms
rtt min/avg/max/mdev = 0.072/0.102/0.125/0.017 ms


Wrote stdout to "docs/output.toml"
```

- `clanix run echo hey`
```
Process ran for 1.036349ms while the timeout was 1ms
Clanix: echo hey failed!
```

- `clanix -t 2 run echo hey` // Failed because the timeout was 1ms, so we made it 2ms
```
Clanix: echo hey was successful!
hey
```

// All these are basic config commands
- `clanix config --init`
- `clanix config --dt true`
- `clanix config -t 2`
- `clanix config --wd docs`
```
Clanix: Successfully set the config.
```

// Verbose mode, will act same for other commands too, just add `-v`
- `clanix -v config --init`
```
Clanix: Successfully set the following config:
ConfigBuilder { disable_timeout: false, timeout_ms: 1, working_dir: "/<redacted>/<redacted>/<redacted>/<redacted>/Clanix" }
```

