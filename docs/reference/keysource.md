# KeySource Object

KeySource objects are used in several places:

* Embedded into DKIM signer objects.
* The `tls_certificate` and `tls_key` fields of listeners.
* To hold credentials for [SMTP AUTH](./kumo/make_egress_path.md#smtp_auth_plain_password).
* With the [kumo.secrets.load](kumo.secrets/load.md) function.

## Acceptable Values

KeySources pattern match from one of the following shapes:

### Local File

When the value is a simple string value, it is interpreted as
the path to a file from which the key will be loaded when needed:

```lua
local file_signer = kumo.dkim.rsa_sha256_signer {
  key = '/path/to/file.pem',
}
```

### Caller Provided Data

When the value is a table with the field `key_data`,
the value of the `key_data` field will be used as the key
data when needed:

```lua
local file_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    -- Doing literally this is probably unwise;
    -- see the example below for a more practical
    -- and manageable way to use this
    key_data = '-----BEGIN RSA PRIVATE KEY----....',
  },
}
```

`key_data` exists to allow you to manage loading key data
via some other lua function, for example, you could load
your keys from a sqlite database:

```lua
function get_key(domain, selector)
  local db = sqlite:open '/opt/kumomta/etc/dkim/keys.db'
  local result = db:execute(
    'select data from keys where domain=? and selector=?',
    domain,
    selector
  )
  return result[1]
end

local sqlite_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    key_data = get_key(msg:from_header().domain, 'default'),
  },
}
```

### HashiCorp Vault

You may store and manage your keys in a [HashiCorp
Vault](https://www.hashicorp.com/products/vault):

```lua
local vault_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    vault_mount = 'secret',
    vault_path = 'dkim/' .. msg:from_header().domain,

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The key must be stored as `key` under the `path` specified.
For example, you might populate it like this:

```console
$ vault kv put -mount=secret dkim/example.org key=@example-private-dkim-key.pem
```
