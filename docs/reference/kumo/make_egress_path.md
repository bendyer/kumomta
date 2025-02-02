# `kumo.make_egress_path { PARAMS }`

Constructs a configuration object that specifies how traffic travelling the
path from a *source* to a *site* will behave.

This function should be called from the
[get_egress_path_config](../events/get_egress_path_config.md) event handler to provide the
configuration for the requested site.

The following keys are possible:

## connection_limit

Specifies the maximum number of concurrent connections that will be made from
the current MTA machine to the destination site.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = 32,
  }
end)
```

## consecutive_connection_failures_before_delay

Each time KumoMTA exhausts the full list of hosts for the destination it
increments a `consecutive_connection_failures` counter. When that counter
exceeds the `consecutive_connection_failures_before_delay` configuration value,
KumoMTA will then delay all of the messages currently in the ready queue,
generating a transient failure log record with code `451 4.4.1 No answer from
any hosts listed in MX`.

The default value for this setting is 100.

## ehlo_domain

Optional string. Specifies the EHLO domain when initiating a connection to
the destination. The default value is the local machine hostname.

## enable_tls

Controls whether and how TLS will be used when connecting to the destination.
Possible values are:

* `"Opportunistic"` - use TLS if advertised by the `EHLO` response. If the peer
  has invalid or self-signed certificates, then the delivery will fail. KumoMTA
  will NOT fallback to not using TLS on that same host.

* `"OpportunisticInsecure"` - use TLS if advertised by the `EHLO` response.
  Validation of the certificate will be skipped. Not recommended for sending to
  the public internet; this is intended for local or lab testing scenarios.

* `"Required"` - Require that TLS be advertised in the `EHLO` response. The
  remote host must have valid certificates in order to deliver to the site.

* `"RequiredInsecure"` - Require that TLS be advertised in the `EHLO` response.
  Validation of the certificate will be skipped.  Not recommended for sending
  to the public internet; this is intended for local or lab testing scenarios.

The default value is `"Opportunistic"`.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
  }
end)
```

## connect_timeout
## starttls_timeout
## ehlo_timeout
## mail_from_timeout
## rcpt_to_timeout
## data_timeout
## data_dot_timeout
## rset_timeout

Controls the timeouts waiting for responses to various SMTP commands.

The value is specified as a integer in seconds, or as a string using syntax
like `"2min"` for a two minute duration.

## idle_timeout
how long a connection will remain open and idle, waiting to be
reused for another delivery attempt, before being closed.

The value is specified as a integer in seconds, or as a string using syntax
like `"2min"` for a two minute duration.


```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    idle_timeout = 60,
  }
end)
```

## max_connection_rate

Optional string.

Specifies the maximum permitted rate at which connections can be established
from this source to the corresponding destination site.

The value is of the form `quantity/period`
where quantity is a number and period can be a measure of time.

Examples of throttles:

```
"10/s" -- 10 per second
"10/sec" -- 10 per second
"10/second" -- 10 per second

"50/m" -- 50 per minute
"50/min" -- 50 per minute
"50/minute" -- 50 per minute

"1,000/hr" -- 1000 per hour
"1_000/h" -- 1000 per hour
"1000/hour" -- 1000 per hour

"10_000/d" -- 10,000 per day
"10,000/day" -- 10,000 per day
```

Throttles are implemented using a Generic Cell Rate Algorithm.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    max_connection_rate = '100/min',
  }
end)
```

If the throttle is exceeded and the delay before a connection be established
is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

## max_deliveries_per_connection

Optional number.

If set, no more than this number of messages will be attempted on any
given connection.

If unset, there is no limit.

## max_message_rate

Optional string.

Specifies the maximum permitted rate at which messages can be delivered
from this source to the corresponding destination site.

The throttle is specified the same was as for `max_connection_rate` above.

If the throttle is exceeded and the delay before the current message can be
sent is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

## max_ready

Specifies the maximum number of messages that can be in the *ready queue*.
The ready queue is the set of messages that are immediately eligible for delivery.

If a message is promoted from its delayed queue to the ready queue and it would
take the size of the ready queue above *max_ready*, the message will be delayed
by a randomized interval of up to 60 seconds before being considered again.

## prohibited_hosts

A CIDR list of hosts that should be considered "poisonous", for example, because
they might cause a mail loop.

When resolving the hosts for the destination MX, if any of the hosts are
present in the `prohibited_hosts` list then the ready queue will be immediately
failed with a `550 5.4.4` status.

## skip_hosts

A CIDR list of hosts that should be removed from the list of hosts returned
when resolving the MX for the destination domain.

This can be used for example to skip a host that is experiencing issues.

If all of the hosts returned for an MX are filtered out by `skip_hosts` then
the ready queue will be immediately failed with a `550 5.4.4` status.

## smtp_port

Specifies the port to connect to when making an SMTP connection to a destination
MX host.

The default is port 25.

See also [kumo.make_egress_source().remote_port](make_egress_source.md#remote_port)

## smtp_auth_plain_username

When set, connecting to the destination requires a successful AUTH PLAIN using the
specified username.

AUTH PLAIN will only be attempted if TLS is also enabled, unless
`allow_smtp_auth_plain_without_tls = true`. This is to prevent leaking
of the credential over an unencrypted link.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'scott',
    -- The password can be any keysource value
    smtp_auth_plain_password = {
      key_data = 'tiger',
    },
  }
end)
```

## smtp_auth_plain_password

Specifies the password that should be used together with `smtp_auth_plain_username`
when an authenticated SMTP connection is desired.

The value is any [keysource](../keysource.md), which allows for specifying the
password inline in the configuration file, or managing it via a credential manager
such as HashiCorp Vault.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'scott',
    -- The password can be any keysource value.
    -- Here we are loading the credential for the domain
    -- from HashiCorp vault
    smtp_auth_plain_password = {
      vault_mount = 'secret',
      vault_path = 'smtp-auth/' .. domain,
    },
  }
end)
```

## allow_smtp_auth_plain_without_tls

Optional boolean. Defaults to `false`.

When `false`, and the connection is not using TLS, SMTP AUTH PLAIN will be
premptively failed in order to prevent the credential from being passed over
the network in clear text.

You can set this to `true` to allow sending the credential in clear text.

!!! danger
    Do not enable this option on an untrusted network, as the credential
    will then be passed in clear text and visible to anyone else on the
    network
