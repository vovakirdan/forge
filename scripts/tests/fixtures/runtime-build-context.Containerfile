# No base image, network, RUN instruction, or executable is needed for this test.
FROM scratch
COPY . /context/
CMD ["/not-executed"]
