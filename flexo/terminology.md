## Terminology

### General terms

#### client-to-server
The connection route between the client that runs pacman, and the "server" that runs flexo. The
server that runs flexo is just a server in the sense that it serves files over HTTP, so we
use this term regardless of whether flexo is running on a "real" server in some remote data center,
or some other kind of hardware like a laptop or a Raspberry Pi.

#### server-to-server
The connection route between flexo and the remote mirror.

#### remote mirror
An official ArchLinux mirror listed in the JSON document: https://archlinux.org/mirrors/status/json/

We use the term "remote mirror" (rather than just "mirror") to distinguish official mirrors from flexo,
because flexo is also a kind of mirror.


### Flexo Library Terms

We aim to implement a large chunk of the code as if it were a reusable library, even though we don't actually intend
to use any of that code as a library. The main reason for doing so is to provide an abstraction layer that we can
reason about without having to consider technical details related to protocols like TCP and HTTP.

Since we have a one-to-one relationship between the entities used in [src/lib.rs](src/lib.rs) and
the entities used in the production version, this document will always describe the two roles
of each entity:
1. The "abstract" role it adopts in [src/lib.rs](src/lib.rs) and all test cases.
2. The actual role it adopts in the production version.

Descriptions about the second role will be formatted as quotes:
> like this.

#### Provider
A provider is required to complete an [order](#order).
> A provider an official Arch Linux mirror. Flexo represents a mirror by its URI,
> For instance, 
> https://mirror.yandex.ru/archlinux/

#### Custom Provider
The usual scenario is that we have a list of providers, and an order can be served by any arbitrary provider from that
list. However, in some cases, the order can only be served by one specific provider.
> A Custom Provider is an [Unofficial Arch Linux User Repository](https://wiki.archlinux.org/index.php/Unofficial_user_repositories),
> for example, `http://repo.archlinux.fr`.
 
#### Order
The order is the information that we require to actually execute a [job](#job).

> An order is the missing path that we mentioned above. For example, 
> the relative path "community/os/x86_64/rustup-1.20.2-1-x86_64.pkg.tar.xz" in combination
> with the provider https://mirror.yandex.ru/archlinux/ can be combined to
> https://mirror.yandex.ru/archlinux/community/os/x86_64/rustup-1.20.2-1-x86_64.pkg.tar.xz,
> which we can then download.

#### Job
A job is the result of combining an order with a provider.
A job can succeed or fail. Failures can have two causes:
1. The order is invalid and therefore impossible to complete successfully, regardless of the provider.
2. The provider is unreliable and therefore not able to complete this order.

The first case is considered a user error which flexo cannot fix. The second case, however, can
be salvaged by trying another provider. It is flexo's primary purpose to
* Select reliable providers in order to reduce the likelihood of failing jobs, and
* provide fast failover to another provider if one job fails.

So if one job fails, we generate a new job by swapping the provider. If all providers
have failed to complete the job, the order has failed to complete.
Since we limit the number of attempts per order, an order can also fail before all providers
have attempted to complete the order.

> A job describes the download process from a given mirror. If the download has failed
> (e.g., due to the server crashing, the server being too slow or just a plain 404 error),
> another mirror is tried.

#### Channel
The channel is the connection to the [provider](#provider). In order to complete an order,
we must first establish a channel to the provider. Creating a new channel requires time
and resources, so we aim to reuse existing channels.
Channels cannot be used for more than one job at a time: If a new job arrives and a channel
to the given provider has already been established, this channel will not be used when it
is still being used by a previous job.

> A channel is a handle that refers to a session.
> Flexo itself is not able to
> recognize whether the connection of this session has already been closed. Instead,
> the handle is able to make all required preparations for subsequent HTTP requests:
> If the connection has been closed, it will reconnect to the server. If it hasn't
> been closed, it will use the existing connection.

