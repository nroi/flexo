FROM nginx:1.19.1

COPY pkg.tar /tmp
RUN tar -C /usr/share/nginx/html -xf /tmp/pkg.tar

RUN truncate -s 31457280 /usr/share/nginx/html/zero

RUN truncate -s 8589934592 /usr/share/nginx/html/large

RUN truncate -s 100 /usr/share/nginx/html/partially-cached

# Files used by flexo_test_resume_offset_not_leaked_across_channel_reuse (regression
# test for issue #93). `partially-cached-leak` is partially cached on the flexo server
# (50 of 100 bytes) so that fetching it triggers an HTTP range request (resume) to the
# mirror; `resume-leak-victim` is a separate, fully-uncached file fetched right after
# over the same reused connection, which must be downloaded in full from byte 0.
RUN truncate -s 100 /usr/share/nginx/html/partially-cached-leak

RUN truncate -s 4096 /usr/share/nginx/html/resume-leak-victim

COPY create_test_files /root
RUN /root/create_test_files
