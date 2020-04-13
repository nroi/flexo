TODO flexo was originally intended to *extend* (rather than replace) cpcache. The document does not
yet reflect this change.

## Terminology
Flexo is not meant to be a reusable library, it is only used by this particular program, called
flexo, which is only meant to be used by another program, called
[cpcache](https://github.com/nroi/cpcache).

There are two reasons why we have created this library (as opposed to just using the curl library
directly, without any additional abstractions in place):
1. Flexo introduces a clear terminology which can be used to explain its purpose. 
2. Flexo introduces an abstraction layer which makes it easier to test and debug
the functionality it provides.

Since we have a one-to-one relationship between the entities used in [src/lib.rs](src/lib.rs) and
the entities used in the production version, this document will always describe the two roles
of each entity:
1. The "abstract" role it adopts in [src/lib.rs](src/lib.rs) and all test cases.
2. The actual role it adopts in the production version.

Descriptions about the second role will be formatted as quotes:
> like this.

### Provider
A provider is required to complete an [order](#order).
> A provider an official Arch Linux mirror. Flexo represents a mirror by its URI,
> For instance, 
> https://mirror.yandex.ru/archlinux/

### Order
The order is the missing information that we require to actually execute a [job](#job).

> A job is the missing path that we mentioned above. For example, 
> the relative path "community/os/x86_64/rustup-1.20.2-1-x86_64.pkg.tar.xz" in combination
> with the provider https://mirror.yandex.ru/archlinux/ can be combined to
> https://mirror.yandex.ru/archlinux/community/os/x86_64/rustup-1.20.2-1-x86_64.pkg.tar.xz,
> which we can then download.

### Job
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

### Channel
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

